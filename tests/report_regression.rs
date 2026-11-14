//! Unified report and explicit regression promotion.

use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_ik"))
}

fn machine() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("examples/publish-approval/machines/publish_approval_v3.ik.toml")
}

fn linear() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/profile/linear.ik.toml")
}

fn arg(path: &Path) -> String {
    path.display().to_string()
}

fn run(args: &[&str]) -> Output {
    bin().args(args).output().expect("spawn ik")
}

fn temp_dir(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "ik-report-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    fs::create_dir_all(&path).expect("temp dir");
    path
}

fn relative_machine(scenario: &Path) -> String {
    let from = scenario
        .parent()
        .expect("scenario dir")
        .canonicalize()
        .expect("scenario dir");
    let to = machine().canonicalize().expect("machine");
    let mut ups = PathBuf::new();
    let mut base = from.as_path();
    loop {
        if let Ok(rest) = to.strip_prefix(base) {
            return ups.join(rest).to_string_lossy().replace('\\', "/");
        }
        ups.push("..");
        base = base.parent().expect("relative machine path");
    }
}

fn write_scenario(dir: &Path, aggregate: &str, steps: Value) -> PathBuf {
    let path = dir.join("scenario.json");
    let body = json!({
        "scenario_version": 1,
        "machine": relative_machine(&path),
        "aggregate": aggregate,
        "steps": steps,
    });
    fs::write(&path, serde_json::to_vec_pretty(&body).expect("scenario")).expect("write");
    path
}

fn stale_steps(expect_stale: &str) -> Value {
    json!([
        {"kind": "command", "label": "edit", "name": "edit", "args": {"artifact_digest": "artifact-1"}, "handle": "cap-edit", "expect": "committed"},
        {"kind": "command", "label": "again", "name": "edit", "args": {"artifact_digest": "artifact-2"}, "expect": "committed"},
        {"kind": "command", "label": "stale", "name": "edit", "args": {"artifact_digest": "artifact-3"}, "use_handle": "cap-edit", "expect": expect_stale}
    ])
}

fn scenario_bundle(dir: &Path, aggregate: &str, expect_stale: &str) -> PathBuf {
    let scenario = write_scenario(dir, aggregate, stale_steps(expect_stale));
    let out = dir.join("bundle");
    let output = run(&[
        "scenario",
        "run",
        &arg(&machine()),
        &arg(&scenario),
        "--out",
        &arg(&out),
    ]);
    assert!(
        out.join("SHA256SUMS").is_file(),
        "bundle missing after {:?}\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
    out
}

fn json_stdout(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|err| {
        panic!(
            "json {err}: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

fn finding<'a>(doc: &'a Value, label: &str) -> &'a Value {
    doc["findings"]
        .as_array()
        .expect("findings")
        .iter()
        .find(|item| item["id"].as_str().unwrap_or("").contains(label))
        .unwrap_or_else(|| panic!("missing {label} in {doc}"))
}

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).expect("copy dest");
    for entry in fs::read_dir(from).expect("read copy") {
        let entry = entry.expect("entry");
        let dest = to.join(entry.file_name());
        if entry.path().is_dir() {
            copy_tree(&entry.path(), &dest);
        } else {
            fs::copy(entry.path(), dest).expect("copy file");
        }
    }
}

fn analysis_result(finding: &str, evidence: &str, complete: bool, reproduction: &str) -> Value {
    json!({
        "protocol": "ik.analysis.result.v1",
        "request_id": "req-1",
        "run_id": "run-1",
        "subject": {"kind": "machine", "identity": {"machine_id": "m", "spec_hash": "abc"}},
        "goals": [{"id": "safe", "content_digest": "digest"}],
        "profile_digest": "profile",
        "inputs": [],
        "execution_status": "completed",
        "properties": [{
            "goal_id": "safe",
            "property_class": "safety",
            "finding": finding,
            "evidence_class": evidence,
            "coverage": {"complete": complete, "reason": "fixture"},
            "reproduction": {"status": reproduction}
        }],
        "artifacts": ["native/stdout.log"]
    })
}

fn write_json(path: &Path, value: &Value) {
    fs::write(path, serde_json::to_vec_pretty(value).expect("json")).expect("write json");
}

#[test]
fn stale_handle_report_labels_refusal_not_a_commit() {
    let dir = temp_dir("stale-report");
    let bundle = scenario_bundle(&dir, "paper-stale", "refused:IK_CAPABILITY_STALE");
    let page = dir.join("report.html");
    let output = run(&["report", &arg(&bundle), "--json", "--html", &arg(&page)]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let doc = json_stdout(&output);
    assert_eq!(doc["schema"], "ik.report.v1");
    let stale = finding(&doc, "/stale");
    let edit = finding(&doc, "/edit");
    assert_eq!(stale["source_kind"], "ik.scenario.result.v1");
    assert_eq!(stale["status"], "refused");
    assert_eq!(stale["committed"], "refused");
    assert_ne!(stale["committed"], "committed");
    assert!(stale["attempted"].as_str().unwrap().contains("artifact-3"));
    assert!(stale["scope"]
        .as_str()
        .unwrap()
        .contains("IK_CAPABILITY_STALE"));
    assert!(stale["scope"].as_str().unwrap().contains("not committed"));
    assert!(!stale["after"].to_string().contains("artifact-3"));
    assert_eq!(
        stale["after"]["generation"],
        finding(&doc, "/again")["after"]["generation"]
    );
    assert_eq!(edit["committed"], "committed");
    assert_eq!(stale["source_coordinate"]["command"], "edit");
    let html = fs::read_to_string(&page).expect("html");
    assert!(html.contains("refused"));
    assert!(!html.to_ascii_lowercase().contains("<script"));
    assert!(!html.contains("http://"));
    assert!(!html.contains("https://"));
}

fn write_scenario_result(dir: &Path, label: &str, outcome: &str) -> PathBuf {
    let path = dir.join(format!("{label}.json"));
    let body = json!({
        "schema": "ik.scenario.result.v1",
        "aggregate": "item",
        "steps": [{
            "label": label,
            "kind": "lookup",
            "verdict": "pass",
            "response": { "outcome": outcome }
        }]
    });
    write_json(&path, &body);
    path
}

#[test]
fn lookup_report_names_history_not_a_new_commit() {
    let dir = temp_dir("lookup-report");
    let cases: &[(&str, &str, &str)] = &[
        (
            "committed_exact",
            "earlier operation: committed (confirmed by lookup); this lookup did not commit again",
            "do not resend; the earlier result stands",
        ),
        (
            "reply_dropped",
            "the reply was discarded; this step did not commit",
            "look up the earlier operation; do not resend from the dropped reply",
        ),
        (
            "not_recorded",
            "no recorded operation matched this lookup; this lookup did not commit",
            "do not treat the earlier attempt as committed from this lookup",
        ),
        (
            "conflict",
            "a record exists but does not match this lookup; this lookup did not commit",
            "do not treat this as the same operation; inspect the conflicting record",
        ),
    ];
    for (outcome, committed, next) in cases {
        let path = write_scenario_result(&dir, outcome, outcome);
        let json_out = run(&["report", "--json", &arg(&path)]);
        let text_out = run(&["report", &arg(&path)]);
        assert_eq!(json_out.status.code(), Some(0), "{outcome}");
        let doc = json_stdout(&json_out);
        let row = finding(&doc, outcome);
        assert_eq!(row["status"], *outcome, "{outcome}");
        assert_eq!(row["committed"], *committed, "{outcome}");
        assert_eq!(row["next_action"], *next, "{outcome}");
        assert_ne!(row["committed"], "not committed", "{outcome}");
        assert!(
            !row["next_action"]
                .as_str()
                .unwrap()
                .contains("inspect status"),
            "{outcome}"
        );
        let text = String::from_utf8_lossy(&text_out.stdout);
        assert!(text.contains(committed), "{outcome}: {text}");
        assert!(text.contains(next), "{outcome}: {text}");
        assert!(
            row["scope"].as_str().unwrap().contains("established:"),
            "{outcome}"
        );
        assert!(
            row["scope"].as_str().unwrap().contains("uncertain:"),
            "{outcome}"
        );
    }
}

#[test]
fn sampled_execution_is_not_reported_as_proven() {
    let dir = temp_dir("sampled");
    let sampled = dir.join("sampled.json");
    let dishonest = dir.join("dishonest.json");
    let exhaustive = dir.join("exhaustive.json");
    write_json(
        &sampled,
        &analysis_result(
            "no_violation_found",
            "sampled_execution",
            true,
            "not_attempted",
        ),
    );
    write_json(
        &dishonest,
        &analysis_result("holds_in_scope", "sampled_execution", true, "model_only"),
    );
    write_json(
        &exhaustive,
        &analysis_result(
            "holds_in_scope",
            "exhaustive_finite_model",
            true,
            "model_only",
        ),
    );
    let output = run(&[
        "report",
        "--json",
        &arg(&sampled),
        &arg(&dishonest),
        &arg(&exhaustive),
    ]);
    assert_eq!(output.status.code(), Some(0));
    let doc = json_stdout(&output);
    let sampled_finding = finding(&doc, "sampled.json");
    let dishonest_finding = finding(&doc, "dishonest.json");
    let exhaustive_finding = finding(&doc, "exhaustive.json");
    assert_eq!(sampled_finding["evidence_class"], "sampled_execution");
    assert_eq!(sampled_finding["committed"], "model-level");
    assert_ne!(sampled_finding["status"], "holds_in_scope");
    assert!(sampled_finding["scope"]
        .as_str()
        .unwrap()
        .contains("sampled execution; not a proof"));
    assert!(!sampled_finding["scope"]
        .as_str()
        .unwrap()
        .contains("proven"));
    assert!(!sampled_finding["scope"]
        .as_str()
        .unwrap()
        .contains("exhaustive"));
    assert_eq!(sampled_finding["reproduction_status"], "not_attempted");
    assert_ne!(dishonest_finding["status"], "holds_in_scope");
    assert!(dishonest_finding["scope"]
        .as_str()
        .unwrap()
        .contains("not a proof"));
    assert!(!dishonest_finding.to_string().contains("proven"));
    assert_eq!(exhaustive_finding["status"], "holds_in_scope");
    assert_eq!(
        exhaustive_finding["evidence_class"],
        "exhaustive_finite_model"
    );
    assert!(exhaustive_finding["scope"]
        .as_str()
        .unwrap()
        .contains("not outside that model"));
    assert_eq!(exhaustive_finding["committed"], "model-level");
}

#[test]
fn incomplete_search_names_its_next_action() {
    let dir = temp_dir("incomplete");
    let profile = dir.join("checks.toml");
    fs::write(
        &profile,
        "profile_version = 1\n[limits]\nmax_depth = 1\n[[goals]]\nid = \"done\"\nkind = \"reach_terminal\"\nterminal = \"done\"\n",
    )
    .expect("profile");
    let check = run(&[
        "check",
        &arg(&linear()),
        "--profile",
        &arg(&profile),
        "--json",
    ]);
    let check_path = dir.join("check.json");
    fs::write(&check_path, &check.stdout).expect("save check");
    let analysis = dir.join("incomplete-analysis.json");
    write_json(
        &analysis,
        &analysis_result("no_witness_found", "bounded_paths", false, "not_attempted"),
    );
    let output = run(&["report", "--json", &arg(&check_path), &arg(&analysis)]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let doc = json_stdout(&output);
    let profile_finding = finding(&doc, "check.json");
    let analysis_finding = finding(&doc, "incomplete-analysis.json");
    assert_eq!(profile_finding["status"], "no_witness_found_incomplete");
    assert_eq!(profile_finding["completeness"], "incomplete");
    assert_ne!(profile_finding["completeness"], "complete");
    assert!(profile_finding["next_action"]
        .as_str()
        .unwrap()
        .contains("raise bounds or add cases"));
    assert!(!profile_finding["scope"]
        .as_str()
        .unwrap()
        .contains("unreachable within the supplied corpus"));
    assert_eq!(analysis_finding["status"], "no_witness_found");
    assert_eq!(analysis_finding["completeness"], "incomplete");
    assert!(analysis_finding["next_action"]
        .as_str()
        .unwrap()
        .contains("raise bounds or add cases"));
    assert_eq!(analysis_finding["evidence_class"], "bounded_paths");
    assert!(!analysis_finding["scope"]
        .as_str()
        .unwrap()
        .contains("proven"));
}

#[test]
fn witness_says_smallest_found_and_coverage_gap_names_the_command() {
    let dir = temp_dir("wording");
    let profile = dir.join("checks.toml");
    fs::write(
        &profile,
        "profile_version = 1\n[[goals]]\nid = \"done\"\nkind = \"reach_terminal\"\nterminal = \"done\"\n",
    )
    .expect("profile");
    let check = run(&[
        "check",
        &arg(&linear()),
        "--profile",
        &arg(&profile),
        "--json",
    ]);
    let check_path = dir.join("check.json");
    fs::write(&check_path, &check.stdout).expect("save check");
    let gap = dir.join("gap.json");
    write_json(
        &gap,
        &json!({
            "verdict": "REFUSED",
            "profile": {
                "schema": "ik.check.profile.v1",
                "search_complete": true,
                "goals": [{
                    "id": "done",
                    "kind": "reach_terminal",
                    "status": "input_coverage_gap",
                    "gaps": [{"command": "go", "classification": "no_corpus_case"}]
                }]
            }
        }),
    );
    let output = run(&["report", "--json", &arg(&check_path), &arg(&gap)]);
    assert_eq!(output.status.code(), Some(0));
    let doc = json_stdout(&output);
    let witness = finding(&doc, "check.json");
    let gap_finding = finding(&doc, "gap.json");
    assert_eq!(witness["status"], "witness_found");
    assert!(witness["scope"]
        .as_str()
        .unwrap()
        .contains("smallest found"));
    assert!(!witness.to_string().contains("minimal"));
    assert_eq!(witness["evidence_class"], "bounded_paths");
    assert_ne!(witness["evidence_class"], "exhaustive_finite_model");
    assert_eq!(gap_finding["status"], "input_coverage_gap");
    assert_eq!(gap_finding["completeness"], "incomplete");
    assert_eq!(gap_finding["next_action"], "add corpus cases for go");
    assert_eq!(gap_finding["source_coordinate"]["command"], "go");
}

#[test]
fn tampered_checksums_block_promote_and_failing_expectations_are_copied() {
    let dir = temp_dir("promote-sums");
    let bundle = scenario_bundle(&dir, "paper-fail", "committed");
    let source_scenario = fs::read(bundle.join("scenario.json")).expect("scenario");
    let source_result = fs::read(bundle.join("result.json")).expect("result");
    let into = dir.join("regressions");
    let promoted = run(&[
        "regression",
        "promote",
        &arg(&bundle),
        "--name",
        "kept-failure",
        "--reason",
        "expectation still says committed",
        "--into",
        &arg(&into),
    ]);
    assert_eq!(
        promoted.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&promoted.stderr)
    );
    let dest = into.join("kept-failure");
    assert_eq!(
        fs::read(dest.join("scenario.json")).unwrap(),
        source_scenario
    );
    assert_eq!(fs::read(dest.join("result.json")).unwrap(), source_result);
    assert_eq!(
        fs::read(bundle.join("scenario.json")).unwrap(),
        source_scenario
    );
    let copied: Value = serde_json::from_slice(&source_scenario).unwrap();
    let stale = copied["steps"]
        .as_array()
        .unwrap()
        .iter()
        .find(|step| step["label"] == "stale")
        .unwrap();
    assert_eq!(stale["expect"], "committed");
    let result: Value = serde_json::from_slice(&source_result).unwrap();
    assert_eq!(result["all_expectations_met"], false);
    let regression: Value =
        serde_json::from_slice(&fs::read(dest.join("REGRESSION.json")).unwrap()).unwrap();
    assert_eq!(regression["reason"], "expectation still says committed");
    assert_eq!(regression["spec_hash"], result["spec_hash"]);
    assert_eq!(
        regression["source_bundle_digest"],
        ik::prelude::sha256_hex(&fs::read(bundle.join("SHA256SUMS")).unwrap())
    );
    assert_eq!(
        regression["runtime_binary_sha256"].as_str().unwrap().len(),
        64
    );

    let tampered = dir.join("tampered-bundle");
    copy_tree(&bundle, &tampered);
    let mut sums = fs::read_to_string(tampered.join("SHA256SUMS")).unwrap();
    let replacement = if sums.starts_with('a') { "b" } else { "a" };
    sums.replace_range(0..1, replacement);
    fs::write(tampered.join("SHA256SUMS"), sums).unwrap();
    let blocked = run(&[
        "regression",
        "promote",
        &arg(&tampered),
        "--name",
        "should-not-land",
        "--reason",
        "tampered",
        "--into",
        &arg(&into),
    ]);
    assert_ne!(blocked.status.code(), Some(0));
    assert_eq!(blocked.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&blocked.stderr).contains("SHA256SUMS"));
    assert!(!into.join("should-not-land").exists());
}

#[test]
fn promote_refuses_to_overwrite() {
    let dir = temp_dir("overwrite");
    let bundle = scenario_bundle(&dir, "paper-keep", "refused:IK_CAPABILITY_STALE");
    let into = dir.join("regressions");
    let first = run(&[
        "regression",
        "promote",
        &arg(&bundle),
        "--name",
        "case",
        "--reason",
        "first reason",
        "--into",
        &arg(&into),
    ]);
    assert_eq!(first.status.code(), Some(0));
    let before = fs::read(into.join("case").join("REGRESSION.json")).unwrap();
    let second = run(&[
        "regression",
        "promote",
        &arg(&bundle),
        "--name",
        "case",
        "--reason",
        "second reason",
        "--into",
        &arg(&into),
    ]);
    assert_eq!(second.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&second.stderr).contains("overwrite"));
    assert_eq!(
        fs::read(into.join("case").join("REGRESSION.json")).unwrap(),
        before
    );
    assert!(!String::from_utf8_lossy(&before).contains("second reason"));
}

#[test]
fn tampered_promoted_scenario_is_not_reproduced() {
    let dir = temp_dir("rerun");
    let bundle = scenario_bundle(&dir, "paper-rerun", "refused:IK_CAPABILITY_STALE");
    let into = dir.join("regressions");
    let promoted = run(&[
        "regression",
        "promote",
        &arg(&bundle),
        "--name",
        "keep",
        "--reason",
        "keep the refusal",
        "--into",
        &arg(&into),
    ]);
    assert_eq!(
        promoted.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&promoted.stderr)
    );
    let good = into.join("keep");
    let bad_root = dir.join("tampered-regressions");
    let bad = bad_root.join("keep");
    copy_tree(&good, &bad);
    let mut scenario: Value =
        serde_json::from_slice(&fs::read(bad.join("scenario.json")).unwrap()).unwrap();
    scenario["steps"][0]["expect"] = json!("refused:IK_CAPABILITY_STALE");
    fs::write(
        bad.join("scenario.json"),
        serde_json::to_vec_pretty(&scenario).unwrap(),
    )
    .unwrap();
    let untampered = run(&["regression", "run", &arg(&good)]);
    let tampered = run(&["regression", "run", &arg(&bad)]);
    let untampered_out = String::from_utf8_lossy(&untampered.stdout);
    let tampered_out = String::from_utf8_lossy(&tampered.stdout);
    assert_eq!(
        untampered.status.code(),
        Some(0),
        "{untampered_out}\n{}",
        String::from_utf8_lossy(&untampered.stderr)
    );
    assert!(untampered_out.contains(": reproduced"));
    assert!(!untampered_out.contains("not_reproduced"));
    assert_eq!(tampered.status.code(), Some(1), "{tampered_out}");
    assert!(tampered_out.contains("not_reproduced"));
}

#[test]
fn unreadable_input_is_visible_and_does_not_drop_other_findings() {
    let dir = temp_dir("unreadable");
    let valid = dir.join("sampled.json");
    write_json(
        &valid,
        &analysis_result(
            "no_violation_found",
            "sampled_execution",
            false,
            "model_only",
        ),
    );
    let garbage = dir.join("garbage.json");
    fs::write(&garbage, b"{").unwrap();
    let missing = dir.join("missing.json");
    let output = run(&[
        "report",
        "--json",
        &arg(&valid),
        &arg(&garbage),
        &arg(&missing),
    ]);
    assert_ne!(output.status.code(), Some(0));
    assert_eq!(output.status.code(), Some(1));
    let doc = json_stdout(&output);
    let valid_finding = finding(&doc, "sampled.json");
    assert_eq!(valid_finding["evidence_class"], "sampled_execution");
    assert_eq!(valid_finding["committed"], "model-level");
    let garbage_finding = finding(&doc, "garbage.json");
    let missing_finding = finding(&doc, "missing.json");
    assert_eq!(garbage_finding["status"], "unreadable");
    assert_eq!(garbage_finding["source_kind"], "unreadable");
    assert!(garbage_finding["scope"]
        .as_str()
        .unwrap()
        .contains("unreadable"));
    assert_eq!(missing_finding["status"], "unreadable");
    assert!(missing_finding["next_action"]
        .as_str()
        .unwrap()
        .contains("not skipped"));
    let text = run(&["report", &arg(&garbage)]);
    assert_ne!(text.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&text.stdout).contains("unreadable"));
}

#[test]
fn unsupported_and_tool_unavailable_name_what_to_configure() {
    let dir = temp_dir("configure");
    let unsupported = dir.join("unsupported.json");
    let missing = dir.join("missing-tool.json");
    write_json(
        &unsupported,
        &json!({
            "protocol": "ik.analysis.result.v1",
            "request_id": "req",
            "run_id": "run",
            "execution_status": "unsupported",
            "properties": [],
            "artifacts": []
        }),
    );
    write_json(
        &missing,
        &json!({
            "protocol": "ik.analysis.host-report.v1",
            "execution_status": "tool_unavailable",
            "rejection_reason": null,
            "detail": "java",
            "findings": [],
            "partial_logs": ["native/stderr.log"]
        }),
    );
    let output = run(&["report", "--json", &arg(&unsupported), &arg(&missing)]);
    assert_eq!(output.status.code(), Some(0));
    let doc = json_stdout(&output);
    let unsupported_finding = finding(&doc, "unsupported.json");
    let missing_finding = finding(&doc, "missing-tool.json");
    assert_eq!(unsupported_finding["status"], "unsupported");
    assert!(unsupported_finding["next_action"]
        .as_str()
        .unwrap()
        .contains("configure the unsupported capability"));
    assert_eq!(missing_finding["status"], "tool_unavailable");
    assert!(missing_finding["next_action"]
        .as_str()
        .unwrap()
        .contains("configure or install the missing tool"));
    assert!(missing_finding["next_action"]
        .as_str()
        .unwrap()
        .contains("java"));
    assert_eq!(missing_finding["reproduction_status"], "not_attempted");
    assert!(missing_finding["artifacts"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item == "native/stderr.log"));
}

#[test]
fn continuity_observation_names_kept_version_working_and_refused_commands() {
    let dir = temp_dir("continuity-report");
    let path = dir.join("result.json");
    write_json(
        &path,
        &json!({
            "schema": "ik.scenario.result.v1",
            "aggregate": "loan-1",
            "steps": [
                {
                    "label": "recall",
                    "kind": "command",
                    "verdict": "pass",
                    "request": { "name": "recall", "args": {} },
                    "response": { "outcome": "committed" },
                    "present": {
                        "created_version": 1,
                        "presented": "prior",
                        "presented_version": 1,
                        "command": "recall"
                    }
                },
                {
                    "label": "extend",
                    "kind": "command",
                    "verdict": "pass",
                    "request": { "name": "extend", "args": { "extra_ms": 1 } },
                    "response": { "outcome": "refused", "diagnostic_code": "IK_MACHINE_MISMATCH" },
                    "present": {
                        "created_version": 1,
                        "presented": "new",
                        "presented_version": 2,
                        "command": "extend",
                        "code": "IK_MACHINE_MISMATCH"
                    }
                }
            ]
        }),
    );
    let json_out = run(&["report", "--json", &arg(&path)]);
    let text_out = run(&["report", &arg(&path)]);
    assert_eq!(json_out.status.code(), Some(0));
    let doc = json_stdout(&json_out);
    let recall = finding(&doc, "/recall");
    let extend = finding(&doc, "/extend");
    assert_eq!(recall["evidence_class"], "continuity_observation");
    assert_eq!(extend["evidence_class"], "continuity_observation");
    let text = String::from_utf8_lossy(&text_out.stdout);
    assert!(
        text.contains("instances created under version 1 keep version 1"),
        "{text}"
    );
    assert!(text.contains("recall still works on them"), "{text}");
    assert!(
        text.contains("extend is refused on them with IK_MACHINE_MISMATCH"),
        "{text}"
    );
    assert!(
        text.contains("existing instances stay on their version; moving one to version 2 in place is not supported"),
        "{text}"
    );
}
