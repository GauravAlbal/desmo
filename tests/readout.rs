use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn ik(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ik")).args(args).output().expect("run ik")
}

fn temp_dir(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("ik-readout-{name}-{}", std::process::id()));
    fs::create_dir_all(&path).expect("dir");
    path
}

/// The delivery handoff the product rendered for the BUILD journey, as committed.
fn real_handoff() -> String {
    handoff_section("### Delivery handoff")
}

fn handoff_section(heading: &str) -> String {
    let readout = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("validation/dfy/human/HUMAN-READOUT.md");
    let text = fs::read_to_string(readout).expect("readout");
    let after = text.split(heading).nth(1).expect("handoff section");
    let block = after.split("~~~~text\n").nth(1).expect("block");
    block.split("\n~~~~").next().expect("end").to_string()
}

fn quote(handoff: &str, starts: &str, through: &str) -> String {
    let lines: Vec<&str> = handoff.lines().collect();
    let from = lines.iter().position(|line| line.starts_with(starts)).expect("start line");
    let to = lines.iter().position(|line| line.starts_with(through)).expect("end line");
    lines[from..to].iter().map(|line| format!("> {line}")).collect::<Vec<_>>().join("\n")
}

/// A paragraph of the handoff from its first line up to the next blank line, as `>` lines.
fn quote_paragraph(handoff: &str, starts: &str) -> String {
    let lines: Vec<&str> = handoff.lines().collect();
    let from = lines.iter().position(|line| line.starts_with(starts)).expect("paragraph start");
    lines[from..].iter().take_while(|line| !line.trim().is_empty()).map(|line| format!("> {line}")).collect::<Vec<_>>().join("\n")
}

fn good_summary(handoff: &str) -> String {
    format!(
        "Delivered the tag flow.\n\n{}\n\n{}\n\n{}\n\nThe `tag $label` argument is unguarded; the owner decides whether that matters.\nThe machine declares no_invariants; accepted for now.\n",
        quote(handoff, "Status:", "Demonstrated"),
        quote(handoff, "Not checked:", "Readiness:"),
        quote_paragraph(handoff, "Claim: "),
    )
}

fn lint(dir: &Path, summary: &str, handoff: &str, extra: &[&str]) -> (Option<i32>, Value, String) {
    let s = dir.join("summary.md");
    let h = dir.join("handoff.txt");
    fs::write(&s, summary).expect("summary");
    fs::write(&h, handoff).expect("handoff");
    let mut args = vec!["readout", s.to_str().expect("s"), "--handoff", h.to_str().expect("h")];
    args.extend_from_slice(extra);
    let text = ik(&args);
    args.push("--json");
    let json = ik(&args);
    (
        text.status.code(),
        serde_json::from_slice(&json.stdout).unwrap_or(Value::Null),
        String::from_utf8_lossy(&text.stdout).into_owned(),
    )
}

fn vectors(doc: &Value) -> Vec<String> {
    doc["defects"].as_array().map(|rows| rows.iter().map(|row| row["vector"].as_str().unwrap_or("").to_string()).collect()).unwrap_or_default()
}

#[test]
fn a_summary_that_quotes_status_and_scope_and_names_the_gaps_is_clean() {
    let dir = temp_dir("clean");
    let handoff = real_handoff();
    let (code, doc, text) = lint(&dir, &good_summary(&handoff), &handoff, &[]);
    assert_eq!(code, Some(0), "{text}");
    assert_eq!(doc["clean"], true);
    assert_eq!(text.contains("not whether the summary is true"), true, "{text}");
}

#[test]
fn each_defect_vector_names_the_line_what_is_wrong_and_the_fix() {
    let dir = temp_dir("vectors");
    let handoff = real_handoff();
    let good = good_summary(&handoff);
    let cases: Vec<(&str, String)> = vec![
        ("readiness_word", format!("{good}\nThis is ready to ship.\n")),
        ("completeness_claim", format!("{good}\nThere is nothing missing.\n")),
        ("proof_word", format!("{good}\nThe tests prove it works.\n")),
        ("quote_not_in_handoff", format!("{good}\n> Status: ALL CLEAR\n")),
        ("status_not_quoted", good.replace("> Status: DECISION REQUIRED\n", "")),
        ("not_checked_not_quoted", good.replace("> Not checked:", "Not checked:")),
        ("claim_not_quoted", good.replace("> Claim: ", "Claim: ")),
        ("gap_not_mentioned", good.replace("$label", "the label").replace("no_invariants", "an unrelated thing")),
    ];
    for (vector, summary) in cases {
        let (code, doc, text) = lint(&dir, &summary, &handoff, &[]);
        assert_eq!(code, Some(1), "{vector}: {text}");
        assert_eq!(vectors(&doc).contains(&vector.to_string()), true, "{vector}: {doc}");
        let row = doc["defects"].as_array().and_then(|rows| rows.iter().find(|row| row["vector"] == vector)).cloned().unwrap_or(Value::Null);
        assert_eq!(row["wrong"].as_str().unwrap_or("").len() > 40, true, "{vector} must say what is wrong: {row}");
        assert_eq!(row["fix"].as_str().unwrap_or("").len() > 20, true, "{vector} must say how to fix: {row}");
        assert_eq!(text.contains(&format!("DEFECT {vector}")), true, "{vector}: {text}");
    }
}

#[test]
fn a_readiness_word_inside_a_verbatim_quote_is_not_a_defect_but_in_prose_it_is_with_its_line() {
    let dir = temp_dir("quote-exempt");
    let handoff = format!("{}\nReadiness: the owner's decision.\n", real_handoff());
    let summary = format!("{}\n\n> Readiness: the owner's decision.\n\nAll complete.\n", good_summary(&handoff));
    let (code, doc, _) = lint(&dir, &summary, &handoff, &[]);
    assert_eq!(code, Some(1));
    let rows = doc["defects"].as_array().cloned().unwrap_or_default();
    assert_eq!(rows.len(), 1, "{doc}");
    assert_eq!(rows[0]["vector"], "readiness_word");
    assert_eq!(rows[0]["line"].as_u64().is_some_and(|line| line > 1), true, "{doc}");
}

#[test]
fn a_stamp_is_written_only_when_clean_and_goes_stale_when_either_text_changes() {
    let dir = temp_dir("stamp");
    let handoff = real_handoff();
    let (code, _, _) = lint(&dir, &format!("{}\nThis is ready.\n", good_summary(&handoff)), &handoff, &["--stamp"]);
    assert_eq!(code, Some(1));
    assert_eq!(fs::read_to_string(dir.join("summary.md")).expect("summary").contains("ikaros-readout"), false, "no stamp on a dirty summary");

    let (code, _, text) = lint(&dir, &good_summary(&handoff), &handoff, &["--stamp"]);
    assert_eq!(code, Some(0), "{text}");
    let stamped = fs::read_to_string(dir.join("summary.md")).expect("stamped");
    assert_eq!(stamped.contains("<!-- ikaros-readout: lint=clean summary="), true, "{stamped}");

    let again = ik(&["readout", dir.join("summary.md").to_str().expect("s"), "--handoff", dir.join("handoff.txt").to_str().expect("h")]);
    assert_eq!(again.status.code(), Some(0), "{}", String::from_utf8_lossy(&again.stdout));

    fs::write(dir.join("summary.md"), stamped.replace("Delivered", "Shipped")).expect("edit summary");
    let edited = ik(&["readout", dir.join("summary.md").to_str().expect("s"), "--handoff", dir.join("handoff.txt").to_str().expect("h")]);
    assert_eq!(edited.status.code(), Some(1));
    assert_eq!(String::from_utf8_lossy(&edited.stdout).contains("DEFECT stamp_stale"), true);

    fs::write(dir.join("summary.md"), &stamped).expect("restore");
    fs::write(dir.join("handoff.txt"), format!("{handoff}\nextra\n")).expect("edit handoff");
    let other = ik(&["readout", dir.join("summary.md").to_str().expect("s"), "--handoff", dir.join("handoff.txt").to_str().expect("h")]);
    assert_eq!(String::from_utf8_lossy(&other.stdout).contains("DEFECT stamp_stale"), true);
}

#[test]
fn a_hand_written_stamp_is_malformed_and_a_non_handoff_file_is_a_usage_error() {
    let dir = temp_dir("forged");
    let handoff = real_handoff();
    let forged = format!("{}\n<!-- ikaros-readout: lint=clean -->\n", good_summary(&handoff));
    let (code, doc, _) = lint(&dir, &forged, &handoff, &[]);
    assert_eq!(code, Some(1));
    assert_eq!(vectors(&doc).contains(&"stamp_malformed".to_string()), true, "{doc}");
    let (code, _, _) = lint(&dir, "x\n", "not a handoff\n", &[]);
    assert_eq!(code, Some(2));
}

#[test]
fn a_recovery_handoff_has_no_status_line_and_is_linted_on_its_scope_paragraph() {
    let dir = temp_dir("recovery");
    let handoff = handoff_section("### Recovery handoff");
    assert_eq!(handoff.contains("\nStatus:"), false, "recovery handoffs carry no Status line");
    let summary = format!("Looked up the recorded operation.\n\n{}\n\n{}\n", quote(&handoff, "Not checked:", "Readiness:"), quote_paragraph(&handoff, "Claim: "));
    let (code, doc, text) = lint(&dir, &summary, &handoff, &[]);
    assert_eq!(code, Some(0), "{text}");
    assert_eq!(doc["clean"], true);
    let (code, doc, _) = lint(&dir, "Looked up the recorded operation.\n", &handoff, &[]);
    assert_eq!(code, Some(1));
    assert_eq!(vectors(&doc), vec!["not_checked_not_quoted".to_string(), "claim_not_quoted".to_string()]);
}

#[test]
fn a_machine_level_gap_is_named_by_its_plain_word_not_the_kind_id() {
    let dir = temp_dir("machine-gap");
    let handoff = real_handoff();
    let good = good_summary(&handoff).replace("no_invariants", "no invariant");
    let (code, _, text) = lint(&dir, &good, &handoff, &[]);
    assert_eq!(code, Some(0), "{text}");
}

#[test]
fn the_recovery_handoff_claim_must_be_quoted_too() {
    let dir = temp_dir("recovery-claim");
    let handoff = handoff_section("### Recovery handoff");
    let without = format!("Looked up the operation.\n\n{}\n", quote(&handoff, "Not checked:", "Readiness:"));
    let (code, doc, _) = lint(&dir, &without, &handoff, &[]);
    assert_eq!(code, Some(1));
    assert_eq!(vectors(&doc), vec!["claim_not_quoted".to_string()]);
}
