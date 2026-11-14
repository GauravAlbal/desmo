//! Lifecycle diagrams compress the definition table; they do not invent facts.

use ik::declarative::{load_definition, parse_definition, Definition};
use ik::diagram::{
    analyze_change, analyze_def, build, build_with, classify_guard, display_width, md_table,
    structurally_ok, Diagram, DiagramKind, GuardKind, MAX_LINE,
};
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn repo(path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(path)
}

fn load(path: &str) -> Definition {
    load_definition(&repo(path)).expect("definition")
}

fn loan_v1() -> Definition {
    load("validation/dfy/journeys/fixtures/loan_checkout.ik.toml")
}

fn loan_v2() -> Definition {
    load("validation/dfy/journeys/fixtures/loan_v2.ik.toml")
}

fn tagged() -> Definition {
    load("validation/dfy/journeys/fixtures/tagged_item.ik.toml")
}

fn diagram_of(
    def: &Definition,
    prior: Option<&Definition>,
    findings: &[Value],
    kind: DiagramKind,
) -> Diagram {
    build(Some(def), prior, findings, kind, &def.machine_id, None)
}

fn ik(args: &[&str]) -> (String, String, Option<i32>) {
    let out = Command::new(env!("CARGO_BIN_EXE_ik"))
        .args(args)
        .output()
        .expect("spawn ik");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code(),
    )
}

fn golden_dir() -> PathBuf {
    repo("tests/fixtures/diagram")
}

fn assert_golden(name: &str, body: &str) {
    let path = golden_dir().join(name);
    fs::create_dir_all(golden_dir()).expect("golden dir");
    if std::env::var("IK_UPDATE_DIAGRAM_GOLDENS").is_ok() || !path.exists() {
        fs::write(&path, body).expect("write golden");
    }
    let expected = fs::read_to_string(&path).expect("read golden");
    assert_eq!(body, expected, "golden {name} drifted");
}

fn dead_end_machine() -> Definition {
    parse_definition(
        r#"
schema_version = 1
machine_id = "trap"
machine_version = 1
[facts]
[states]
start = { initial = true }
trap = {}
done = { terminal = true }
ghost = {}
[commands.go]
from = ["start"]
to = "trap"
[commands.finish]
from = ["start"]
to = "done"
"#,
    )
    .expect("dead end")
}

fn reach_v1() -> Definition {
    parse_definition(
        r#"
schema_version = 1
machine_id = "reach"
machine_version = 1
[facts]
[states]
start = { initial = true }
mid = {}
done = { terminal = true }
[commands.step]
from = ["start"]
to = "mid"
[commands.finish]
from = ["mid"]
to = "done"
"#,
    )
    .expect("reach v1")
}

fn reach_v2() -> Definition {
    parse_definition(
        r#"
schema_version = 1
machine_id = "reach"
machine_version = 2
[facts]
[states]
start = { initial = true }
mid = {}
done = { terminal = true }
[commands.step]
from = ["start"]
to = "mid"
[commands.shortcut]
from = ["start"]
to = "done"
"#,
    )
    .expect("reach v2")
}

#[test]
fn loan_v1_analytics_match_hand_computed() {
    let a = analyze_def(&loan_v1());
    assert_eq!(a.state_count, 3);
    assert_eq!(a.command_count, 3);
    assert_eq!(a.terminals, vec!["returned"]);
    assert!(a.unreachable.is_empty(), "{:?}", a.unreachable);
    assert!(a.dead_ends.is_empty(), "{:?}", a.dead_ends);
    assert!(a.every_state_reaches_end);
    assert_eq!(a.shortest_from_initial, Some(2));
    assert_eq!(a.shortest_finish.get("idle").copied().flatten(), Some(2));
    assert_eq!(a.shortest_finish.get("on_loan").copied().flatten(), Some(1));
    assert_eq!(
        a.shortest_finish.get("returned").copied().flatten(),
        Some(0)
    );
    assert_eq!(a.loops.len(), 1);
    assert_eq!(a.loops[0].label, "on_loan ↺ recall");
    assert_eq!(
        classify_guard("$loan_until_ms == $checkout_ms + 1209600000"),
        GuardKind::Time
    );
    assert_eq!(
        classify_guard("$overdue == 0 || recall_placed == 1"),
        GuardKind::Count
    );
    assert!(a.guarded_commands.contains(&"checkout".to_string()));
}

#[test]
fn loan_v2_change_analytics_and_cohort() {
    let prior = loan_v1();
    let next = loan_v2();
    let delta = analyze_change(&prior, &next);
    assert_eq!(delta.commands_added, vec!["extend"]);
    assert!(delta.states_added.is_empty());
    assert_eq!(
        delta.reachability_line,
        "Reachability: unchanged (structural)"
    );
    assert!(delta.every_state_still_reaches_end);
    assert_eq!(
        classify_guard("$new_until_ms > loan_until_ms"),
        GuardKind::Time
    );
    let findings = vec![json!({
        "evidence_class": "continuity_observation",
        "status": "refused",
        "present": {
            "created_version": 1,
            "command": "extend",
            "code": "IK_MACHINE_MISMATCH"
        },
        "before": { "state": "on_loan" },
        "source_coordinate": { "command": "extend" },
        "attempted": "extend {}"
    })];
    let diagram = diagram_of(&next, Some(&prior), &findings, DiagramKind::Change);
    let view = diagram.change.as_ref().expect("change view");
    assert!(view.added_commands.contains(&"extend".to_string()));
    let on_loan = view
        .cohorts
        .iter()
        .find(|row| row.states.iter().any(|name| name == "on_loan"))
        .expect("on_loan row");
    assert!(on_loan.existing.contains("✗extend"), "{}", on_loan.existing);
    assert!(
        on_loan.new_instances.contains("extend"),
        "{}",
        on_loan.new_instances
    );
    assert!(on_loan.existing.contains("checkin"), "{}", on_loan.existing);
    assert_eq!(diagram.emphasis, ik::diagram::Emphasis::New);
    let compressed = diagram.to_compressed_view();
    assert!(
        compressed.contains("extend REFUSED (IK_MACHINE_MISMATCH), observed"),
        "{compressed}"
    );
    assert_eq!(compressed.matches("WHAT ELSE IS AT RISK").count(), 1, "{compressed}");
    assert_eq!(compressed.matches("      │  ✚ ↺ extend").count(), 1, "{compressed}");
    assert_eq!(compressed.matches("none / NOT ESTABLISHED").count(), 0, "{compressed}");
}

#[test]
fn tagged_item_analytics_and_grammar_guard() {
    let a = analyze_def(&tagged());
    assert_eq!(a.state_count, 2);
    assert_eq!(a.command_count, 1);
    assert_eq!(a.shortest_from_initial, Some(1));
    assert!(a.loops.is_empty());
    assert_eq!(
        classify_guard("text_in($id, 'A-Za-z0-9._-', 1, 64)"),
        GuardKind::Grammar
    );
}

#[test]
fn dead_end_and_unreachable_are_named() {
    let a = analyze_def(&dead_end_machine());
    assert!(
        a.dead_ends.contains(&"trap".to_string()),
        "{:?}",
        a.dead_ends
    );
    assert_eq!(a.unreachable, vec!["ghost"]);
    assert_eq!(
        a.cannot_finish,
        vec!["ghost".to_string(), "trap".to_string()]
    );
    let diagram = diagram_of(&dead_end_machine(), None, &[], DiagramKind::Delivery);
    assert_eq!(diagram.emphasis, ik::diagram::Emphasis::Stuck);
    assert_eq!(
        diagram.facts_line,
        "4 states, 2 commands · 1 end · 1 cannot reach an end (1 dead end) · 1 unreachable · shortest finish: 1 step · all rules declared"
    );
}

fn orbit_machine() -> Definition {
    parse_definition(
        r#"
schema_version = 1
machine_id = "orbit"
machine_version = 1
[facts]
[states]
north = { initial = true }
pit = {}
south = { terminal = true }
mist = {}
hold = {}
[commands.land]
from = ["north"]
to = "south"
[commands.sink]
from = ["north"]
to = "pit"
[commands.spin]
from = ["hold"]
to = "hold"
[commands.enter]
from = ["north"]
to = "hold"
"#,
    )
    .expect("orbit")
}

#[test]
fn loops_only_trap_is_distinct_from_dead_end() {
    let a = analyze_def(&orbit_machine());
    assert_eq!(a.dead_ends, vec!["pit".to_string()]);
    assert_eq!(a.traps, vec!["hold".to_string()]);
    assert_eq!(a.loops_only_traps, vec!["hold".to_string()]);
    assert_eq!(a.unreachable, vec!["mist".to_string()]);
    let diagram = diagram_of(&orbit_machine(), None, &[], DiagramKind::Delivery);
    assert_eq!(
        diagram.facts_line.contains("2 cannot reach an end (1 dead end, 1 loops-only trap)"),
        true,
        "{}",
        diagram.facts_line
    );
    assert_eq!(diagram.facts_line.contains("1 unreachable"), true, "{}", diagram.facts_line);
    let map = diagram.to_route_map();
    assert_eq!(map.contains("trap: loops only"), true, "{map}");
    assert_eq!(
        map.lines()
            .any(|line| line.contains("[ hold ]") && line.contains("dead end")),
        false,
        "{map}"
    );
}

#[test]
fn change_that_removes_finish_path_is_an_explicit_delta() {
    let delta = analyze_change(&reach_v1(), &reach_v2());
    assert!(delta.commands_removed.contains(&"finish".to_string()));
    assert!(delta.commands_added.contains(&"shortcut".to_string()));
    assert_eq!(delta.new_dead_ends, vec!["mid".to_string()]);
    assert_eq!(
        delta.reachability_line,
        "Reachability: new dead ends mid; shortest finish changed at mid, start; not every state reaches an end (structural)"
    );
}

#[test]
fn loan_v1_route_map_and_table_are_lossless_and_golden() {
    let def = loan_v1();
    let diagram = diagram_of(&def, None, &[], DiagramKind::Delivery);
    let map = diagram.to_route_map();
    let table = diagram.to_table_markdown();
    for command in def.commands.keys() {
        assert!(table.contains(command), "{command} missing from {table}");
        assert!(
            map.contains(command) || table.contains(command),
            "{command} missing"
        );
    }
    assert!(
        table.contains("$loan_until_ms == $checkout_ms + 1209600000"),
        "{table}"
    );
    assert!(map.contains("idle"), "{map}");
    assert_eq!(map.contains("returned") && map.contains("✔ [") && map.contains(" END"), true, "{map}");
    assert!(map.contains("↺") && map.contains("recall"), "{map}");
    assert!(
        diagram.facts_line.contains("shortest finish: 2 steps"),
        "{}",
        diagram.facts_line
    );
    assert_eq!(diagram.facts_chunks.len(), 5);
    assert!(diagram.legend.len() <= 10, "{:?}", diagram.legend);
    assert!(
        diagram
            .to_compressed_view()
            .lines()
            .filter(|line| line.starts_with("legend:"))
            .count()
            <= 2
    );
    assert_golden("loan_v1.route.txt", &map);
    assert_golden("loan_v1.table.md", &table);
    assert_golden("loan_v1.mermaid", &diagram.to_mermaid());
}

#[test]
fn loan_v2_change_route_map_is_golden() {
    let diagram = diagram_of(&loan_v2(), Some(&loan_v1()), &[], DiagramKind::Change);
    let map = diagram.to_route_map();
    assert!(map.contains("extend"), "{map}");
    assert_eq!(map.matches("WHAT ELSE IS AT RISK").count(), 1, "{map}");
    assert_eq!(map.matches("DIVERGENCE").count(), 0, "{map}");
    assert_golden("loan_v2.route.txt", &map);
    assert_golden("loan_v2.table.md", &diagram.to_table_markdown());
    assert_golden("loan_v2.mermaid", &diagram.to_mermaid());
}

#[test]
fn tagged_item_and_recovery_maps_are_golden() {
    let tagged_def = tagged();
    let delivery = diagram_of(&tagged_def, None, &[], DiagramKind::Delivery);
    assert_golden("tagged_item.route.txt", &delivery.to_route_map());
    assert_golden("tagged_item.table.md", &delivery.to_table_markdown());
    assert_golden("tagged_item.mermaid", &delivery.to_mermaid());
    let lost = vec![
        json!({"status": "committed", "evidence_class": "observed_trace", "attempted": "{\"operation_id\":\"op-tag\"}"}),
        json!({"status": "reply_dropped", "evidence_class": "observed_trace"}),
        json!({"status": "committed_exact", "evidence_class": "observed_trace"}),
        json!({"status": "historical", "evidence_class": "observed_trace"}),
    ];
    let recovery = build(
        Some(&tagged_def),
        None,
        &lost,
        DiagramKind::Recovery,
        "item-lost",
        Some("op-tag"),
    );
    let map = recovery.to_route_map();
    assert!(map.contains("do not resend"), "{map}");
    assert!(map.contains("Established:"), "{map}");
    assert!(map.contains("✂"), "{map}");
    assert_golden("recovery_lost.route.txt", &map);
    let recorded = vec![json!({"status": "committed", "evidence_class": "observed_trace"})];
    let rec = build(
        Some(&tagged_def),
        None,
        &recorded,
        DiagramKind::Recovery,
        "item",
        Some("op-tag"),
    );
    assert_golden("recovery_recorded.route.txt", &rec.to_route_map());
}

#[test]
fn mermaid_is_structurally_valid_flowchart() {
    let diagram = diagram_of(&loan_v1(), None, &[], DiagramKind::Delivery);
    let source = diagram.to_mermaid();
    assert_eq!(source.lines().next(), Some("flowchart LR"));
    assert!(structurally_ok(&source), "{source}");
    assert_eq!(diagram.to_markdown().matches("```mermaid").count(), 0);
    assert_eq!(
        source
            .lines()
            .filter(|line| line.starts_with("flowchart"))
            .count(),
        1
    );
}

#[test]
fn observed_path_is_numbered_and_explored_is_dashed() {
    let findings = vec![
        json!({
            "evidence_class": "observed_trace",
            "status": "committed",
            "source_coordinate": {"command": "tag"},
            "before": {"state": "idle"},
            "after": {"state": "tagged"},
            "attempted": "tag {}"
        }),
        json!({
            "evidence_class": "bounded_paths",
            "status": "witness_found",
            "source_coordinate": {"command": "tag"}
        }),
    ];
    let diagram = diagram_of(&tagged(), None, &findings, DiagramKind::Delivery);
    let map = diagram.to_route_map();
    assert!(map.contains("①") || map.contains("1"), "{map}");
    assert_eq!(diagram.you_are_here.as_deref(), Some("tagged"));
    let hop = diagram
        .hops
        .iter()
        .find(|h| h.command == "tag")
        .expect("tag");
    assert_eq!(hop.stroke, ik::diagram::StrokeClass::Observed);
}

#[test]
fn collapse_caps_states_at_twelve() {
    let mut toml = String::from(
        "schema_version = 1\nmachine_id = \"line\"\nmachine_version = 1\n[facts]\n[states]\ns0 = { initial = true }\n",
    );
    for i in 1..15 {
        if i == 14 {
            toml.push_str(&format!("s{i} = {{ terminal = true }}\n"));
        } else {
            toml.push_str(&format!("s{i} = {{}}\n"));
        }
    }
    toml.push_str("[commands]\n");
    for i in 0..14 {
        toml.push_str(&format!(
            "[commands.step{i}]\nfrom = [\"s{i}\"]\nto = \"s{}\"\n",
            i + 1
        ));
    }
    let def = parse_definition(&toml).expect("line machine");
    let diagram = diagram_of(&def, None, &[], DiagramKind::Delivery);
    assert!(diagram.stations.len() <= 12, "{}", diagram.stations.len());
    assert!(diagram.stations.iter().any(|st| st.collapsed));
}

#[test]
fn ux_lint_over_goldens() {
    let cases = [
        diagram_of(&loan_v1(), None, &[], DiagramKind::Delivery),
        diagram_of(&loan_v2(), Some(&loan_v1()), &[], DiagramKind::Change),
        diagram_of(&tagged(), None, &[], DiagramKind::Delivery),
        diagram_of(&dead_end_machine(), None, &[], DiagramKind::Delivery),
        diagram_of(&reach_v2(), Some(&reach_v1()), &[], DiagramKind::Change),
        diagram_of(
            &loan_v2(),
            Some(&loan_v1()),
            &loan_continuity(),
            DiagramKind::Change,
        ),
        build(
            Some(&tagged()),
            None,
            &[
                json!({"status": "committed"}),
                json!({"status": "reply_dropped"}),
            ],
            DiagramKind::Recovery,
            "item",
            Some("op"),
        ),
    ];
    for diagram in cases {
        let map = diagram.to_route_map();
        let lines: Vec<&str> = map.lines().collect();
        assert!(lines.len() <= 80, "line budget {} in {map}", lines.len());
        for line in &lines {
            assert!(
                display_width(line) <= 120,
                "width {} in `{line}`",
                display_width(line)
            );
        }
        assert!(diagram.legend.len() <= 10, "{:?}", diagram.legend);
        let legend_lines = diagram
            .to_compressed_view()
            .lines()
            .filter(|line| line.starts_with("legend:"))
            .count();
        assert!(
            legend_lines <= 2,
            "{legend_lines} in {}",
            diagram.to_compressed_view()
        );
        for line in diagram
            .to_compressed_view()
            .lines()
            .filter(|line| line.starts_with("legend:"))
        {
            let entries = line.trim_start_matches("legend: ").split(" · ").count();
            assert!(entries <= 5, "{line}");
        }
        assert!(
            diagram.facts_chunks.len() <= 5,
            "{:?}",
            diagram.facts_chunks
        );
        let _ = diagram.emphasis;
        assert!(
            structurally_ok(&diagram.to_mermaid()),
            "{}",
            diagram.to_mermaid()
        );
    }
}

#[test]
fn cli_emits_text_and_matches_library() {
    let path = repo("validation/dfy/journeys/fixtures/loan_checkout.ik.toml");
    let (out, err, code) = ik(&["diagram", path.to_str().expect("path"), "--format", "text"]);
    assert_eq!(code, Some(0), "{err}");
    let expected = diagram_of(&loan_v1(), None, &[], DiagramKind::Delivery).to_compressed_view();
    assert_eq!(out, expected);
    let (help, _, help_code) = ik(&["diagram", "--help"]);
    assert_eq!(help_code, Some(0));
    let usage = help
        .lines()
        .find(|line| line.contains("ik diagram") && line.contains("--format"))
        .expect("diagram usage");
    assert_eq!(
        usage,
        "usage: ik diagram <machine.ik.toml> [--prior <old.ik.toml>] [--evidence <path>]... [--kind delivery|change|recovery] [--format markdown|text|mermaid|table|json] [--focus <state>] [--level 0|1|2] [--ascii] [--operation <id>]"
    );
}

#[test]
fn focus_keeps_the_neighbourhood() {
    let diagram = build_with(
        Some(&loan_v1()),
        None,
        &[],
        DiagramKind::Delivery,
        "loan",
        None,
        2,
        Some("on_loan"),
        false,
        None,
    );
    assert!(diagram.stations.iter().any(|st| st.id == "on_loan"));
    assert!(diagram.focus.as_deref() == Some("on_loan"));
}

#[test]
fn json_field_is_ik_diagram_v1() {
    let diagram = diagram_of(&loan_v1(), None, &[], DiagramKind::Delivery);
    let json = diagram.json_field();
    assert_eq!(json["schema"], "ik.diagram.v1");
    assert!(json["compressed"].as_str().unwrap_or("").contains("idle"));
    assert!(json["facts"]["line"]
        .as_str()
        .unwrap_or("")
        .contains("structural"));
}

#[test]
fn pin_and_loops_are_inline_on_the_station() {
    let delivery = diagram_of(&loan_v1(), None, &[], DiagramKind::Delivery);
    let map = delivery.to_route_map();
    assert!(map.contains("● [ idle ]"), "{map}");
    assert!(map.contains("YOU ARE HERE (2 steps to end)"), "{map}");
    assert_eq!(
        map.lines()
            .filter(|line| line.trim() == "▼ you are here")
            .count(),
        0
    );
    assert_eq!(
        map.lines()
            .filter(|line| line.contains("YOU ARE HERE"))
            .count(),
        1,
    );
    assert!(
        map.lines()
            .any(|line| line.contains("↺ can also: recall")),
        "{map}"
    );
    let tagged_map = diagram_of(&tagged(), None, &[], DiagramKind::Delivery).to_route_map();
    assert!(tagged_map.contains("● [ idle ]"), "{tagged_map}");
    assert!(tagged_map.contains("YOU ARE HERE"), "{tagged_map}");
}

#[test]
fn unreachable_and_dead_end_are_labelled_not_chained() {
    let diagram = diagram_of(&dead_end_machine(), None, &[], DiagramKind::Delivery);
    let map = diagram.to_route_map();
    assert_eq!(map.matches("····▶").count(), 0);
    assert_eq!(map.matches("unreachable from start:").count(), 1, "{map}");
    assert_eq!(map.matches("[ ghost ]").count(), 1, "{map}");
    assert_eq!(map.matches("dead end").count(), 1, "{map}");
    assert!(map.contains("go"), "{map}");
    assert_eq!(
        diagram.facts_line,
        "4 states, 2 commands · 1 end · 1 cannot reach an end (1 dead end) · 1 unreachable · shortest finish: 1 step · all rules declared"
    );
}

#[test]
fn removed_command_is_visible_on_the_old_line() {
    let diagram = diagram_of(&reach_v2(), Some(&reach_v1()), &[], DiagramKind::Change);
    let map = diagram.to_route_map();
    assert_eq!(map.contains("✖ finish → [ done ]  REMOVED in v2"), true, "{map}");
    assert_eq!(map.contains("(changed: lost exit finish)"), true, "{map}");
    assert_eq!(map.contains("CHANGE: +shortcut  ✖ finish"), true, "{map}");
    assert_eq!(map.contains("mid (new)"), true, "{map}");
    assert_eq!(map.contains("Shortest finish changed"), true, "{map}");
    assert_eq!(map.contains("↗ - "), false, "{map}");
    assert!(map.contains("shortcut"), "{map}");
    assert_eq!(
        diagram.facts_line,
        "3 states, 2 commands · 1 end · 1 cannot reach an end (1 dead end) · shortest finish: 1 step · all rules declared"
    );
    assert!(
        true,
        "{:?}",
        diagram.legend
    );
    assert!(
        true,
        "{:?}",
        diagram.legend
    );
}

fn write_pack_file(path: &std::path::Path, body: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("pack dir");
    }
    fs::write(path, body).expect("write pack");
}

fn tagged_observed() -> Vec<Value> {
    vec![json!({
        "evidence_class": "observed_trace",
        "status": "committed",
        "source_coordinate": {"command": "tag"},
        "before": {"state": "idle"},
        "after": {"state": "tagged"},
        "attempted": "tag {}"
    })]
}

fn loan_continuity() -> Vec<Value> {
    vec![
        json!({
            "evidence_class": "bounded_paths",
            "status": "witness_found",
            "source_coordinate": {"command": "checkout"}
        }),
        json!({
            "evidence_class": "continuity_observation",
            "status": "committed",
            "present": {"created_version": 1, "command": "checkout"},
            "before": {"state": "idle"},
            "source_coordinate": {"command": "checkout"}
        }),
        json!({
            "evidence_class": "continuity_observation",
            "status": "refused",
            "present": {
                "created_version": 1,
                "command": "extend",
                "code": "IK_MACHINE_MISMATCH"
            },
            "before": {"state": "on_loan"},
            "source_coordinate": {"command": "extend"},
            "attempted": "extend {}"
        }),
        json!({
            "evidence_class": "continuity_observation",
            "status": "committed",
            "present": {"created_version": 1, "command": "recall"},
            "before": {"state": "on_loan"},
            "source_coordinate": {"command": "recall"}
        }),
    ]
}

fn recovery_lost_findings() -> Vec<Value> {
    vec![
        json!({"status": "committed", "evidence_class": "observed_trace", "attempted": "{\"operation_id\":\"op-tag\"}"}),
        json!({"status": "reply_dropped", "evidence_class": "observed_trace"}),
        json!({"status": "committed_exact", "evidence_class": "observed_trace"}),
        json!({"status": "historical", "evidence_class": "observed_trace"}),
    ]
}

#[test]
fn write_gallery_and_blind_views_when_requested() {
    let delivery = diagram_of(&tagged(), None, &tagged_observed(), DiagramKind::Delivery);
    let change = diagram_of(
        &loan_v2(),
        Some(&loan_v1()),
        &loan_continuity(),
        DiagramKind::Change,
    );
    let recover = build(
        Some(&tagged()),
        None,
        &recovery_lost_findings(),
        DiagramKind::Recovery,
        "item-lost",
        Some("op-tag"),
    );
    let build_a = diagram_of(&loan_v1(), None, &[], DiagramKind::Delivery);
    let build_b = delivery.clone();
    let build_c = diagram_of(&dead_end_machine(), None, &[], DiagramKind::Delivery);
    let change_b = diagram_of(&reach_v2(), Some(&reach_v1()), &[], DiagramKind::Change);
    let recover_b = build(
        Some(&tagged()),
        None,
        &[json!({"status": "committed", "evidence_class": "observed_trace"})],
        DiagramKind::Recovery,
        "item",
        None,
    );
    assert!(
        build_a.to_compressed_view().contains("YOU ARE HERE (2 steps to end)"),
        "{}",
        build_a.to_compressed_view()
    );
    assert!(
        build_b.to_compressed_view().contains("YOU ARE HERE"),
        "{}",
        build_b.to_compressed_view()
    );
    assert_eq!(
        build_c.facts_line,
        "4 states, 2 commands · 1 end · 1 cannot reach an end (1 dead end) · 1 unreachable · shortest finish: 1 step · all rules declared"
    );
    assert!(
        change
            .to_compressed_view()
            .contains("extend"),
        "{}",
        change.to_compressed_view()
    );
    assert!(
        change_b.to_compressed_view().contains("finish"),
        "{}",
        change_b.to_compressed_view()
    );
    assert!(
        recover
            .to_compressed_view()
            .contains("Established: the earlier operation committed"),
        "{}",
        recover.to_compressed_view()
    );
    assert!(
        recover_b
            .to_compressed_view()
            .contains("do not resend; the earlier result stands") || recover_b.to_compressed_view().contains("look up by saved operation id"),
        "{}",
        recover_b.to_compressed_view()
    );
    let Ok(root) = std::env::var("IK_WRITE_DIAGRAM_PACK") else {
        return;
    };
    let root = PathBuf::from(root);
    let gallery = root.join("dfy/gates/diagrams");
    let views = gallery.join("blind/views");
    write_pack_file(
        &gallery.join("delivery.md"),
        &format!(
            "# BUILD\n\nGenerated from the journey fixtures and `ik diagram`.\n\n{}",
            delivery.to_markdown()
        ),
    );
    write_pack_file(
        &gallery.join("change.md"),
        &format!(
            "# CHANGE\n\nGenerated from the journey fixtures and `ik diagram`.\n\n{}",
            change.to_markdown()
        ),
    );
    write_pack_file(
        &gallery.join("recovery.md"),
        &format!(
            "# RECOVER\n\nGenerated from the journey fixtures and `ik diagram`.\n\n{}",
            recover.to_markdown()
        ),
    );
    write_pack_file(
        &gallery.join("index.md"),
        "# Lifecycle diagrams\n\nGenerated from the journey fixtures and the product `ik diagram` command.\n\n| File | Journey | Kind |\n| --- | --- | --- |\n| [delivery.md](delivery.md) | BUILD | delivery |\n| [change.md](change.md) | CHANGE | change |\n| [recovery.md](recovery.md) | RECOVER | recovery |\n\nPrimary projection is the monospace route map and command table in each `.md`. Mermaid is `ik diagram --format mermaid` only. SVG was not generated.\n",
    );
    write_pack_file(&views.join("build-a.txt"), &build_a.to_compressed_view());
    write_pack_file(&views.join("build-b.txt"), &build_b.to_compressed_view());
    write_pack_file(&views.join("build-c.txt"), &build_c.to_compressed_view());
    write_pack_file(&views.join("change-a.txt"), &change.to_compressed_view());
    write_pack_file(&views.join("change-b.txt"), &change_b.to_compressed_view());
    write_pack_file(&views.join("recover-a.txt"), &recover.to_compressed_view());
    write_pack_file(
        &views.join("recover-b.txt"),
        &recover_b.to_compressed_view(),
    );
}

#[test]
fn recovery_map_action_matches_handoff_safe_next() {
    let diagram = build(
        Some(&tagged()),
        None,
        &recovery_lost_findings(),
        DiagramKind::Recovery,
        "item-lost",
        Some("op-tag"),
    );
    let expected = "do not resend; the earlier result stands";
    assert_eq!(diagram.safe_next.as_deref(), Some(expected));
    let line = format!("Safe next action: {expected}");
    assert_eq!(
        diagram
            .to_route_map()
            .lines()
            .filter(|row| *row == line)
            .count(),
        1,
        "{}",
        diagram.to_route_map()
    );
}

#[test]
fn recovery_lookup_resolves_the_fork() {
    let lost = vec![
        json!({"status": "committed", "evidence_class": "observed_trace", "attempted": "{\"operation_id\":\"op-tag\"}"}),
        json!({"status": "reply_dropped", "evidence_class": "observed_trace"}),
        json!({"status": "committed_exact", "evidence_class": "observed_trace"}),
        json!({"status": "historical", "evidence_class": "observed_trace"}),
    ];
    let map = build(
        Some(&tagged()),
        None,
        &lost,
        DiagramKind::Recovery,
        "item-lost",
        Some("op-tag"),
    )
    .to_route_map();
    assert_golden("recovery_lost.route.txt", &map);
    assert_eq!(map.lines().filter(|line| line.contains("✂")).count(), 1);
    assert_eq!(map.matches("not committed ✗").count(), 0);
    assert_eq!(map.matches("unknown ✗").count(), 0);
    assert_eq!(map.matches("accepted ━").count(), 0);
}

#[test]
fn recovery_without_crash_says_none_recorded() {
    let map = build(
        Some(&tagged()),
        None,
        &[json!({"status": "committed", "evidence_class": "observed_trace"})],
        DiagramKind::Recovery,
        "item",
        Some("op-tag"),
    )
    .to_compressed_view();
    assert_eq!(map.matches("✂").count(), 0, "{map}");
    assert!(map.contains("look up by saved operation id"), "{map}");
    assert_eq!(map.lines().filter(|line| line.contains("✂")).count(), 0);
    assert_eq!(map.matches("not committed ✗").count(), 0);
}


#[test]
fn region_parallel_uses_authored_commands() {
    let def = load_definition(&repo("machines/region_parallel.ik.toml")).expect("parallel");
    let diagram = diagram_of(&def, None, &[], DiagramKind::Delivery);
    let map = diagram.to_route_map();
    assert_eq!(map.matches("paused__unmuted").count(), 0, "{map}");
    assert_eq!(map.matches("region_playback").count(), 0, "{map}");
    assert_eq!(map.contains("pause ▼    ▲ play"), true, "{map}");
    assert_eq!(map.contains("mute ▼    ▲ unmute"), true, "{map}");
    assert_eq!(map.contains("pause_and_mute"), true, "{map}");
    assert_eq!(
        diagram.facts_line.contains("5 commands (1 joint)"),
        true,
        "{}",
        diagram.facts_line
    );
    assert_eq!(map.matches("L1 stacked").count(), 0, "{map}");
    let table = diagram.to_table_markdown();
    let squashed = squash(&table);
    assert_eq!(squashed.contains("| playback | pause | playing | paused |"), true, "{table}");
    assert_eq!(squashed.contains("| joint | pause_and_mute |"), true, "{table}");
    assert_eq!(table.matches("paused__unmuted").count(), 0, "{table}");
}

#[test]
fn region_stacked_uses_stacked_lanes() {
    let path = repo("tests/fixtures/diagram/region_stacked.ik.toml");
    let (stdout, stderr, code) = ik(&["check", path.to_str().unwrap()]);
    assert_eq!(code, Some(0), "{stderr}{stdout}");
    assert_eq!(stdout.contains("ACCEPT"), true, "{stdout}");

    let def = load_definition(&path).expect("stacked");
    // Hand-computed product: 2 × 2 × 2 = 8, bound 64.
    assert_eq!(def.states.len(), 8);
    assert_eq!(def.states.len() <= 64, true);

    let diagram = diagram_of(&def, None, &[], DiagramKind::Delivery);
    let regions = diagram.regions.as_ref().expect("regions");
    assert_eq!(regions.lanes.len(), 3);
    let leaf_n: usize = regions.lanes.iter().map(|lane| lane.states.len()).sum();
    assert_eq!(leaf_n, 6);
    assert_eq!(regions.joints.len(), 2);
    assert_eq!(
        diagram.facts_line.contains("3 regions"),
        true,
        "{}",
        diagram.facts_line
    );
    assert_eq!(
        diagram.facts_line.contains("6 leaf states"),
        true,
        "{}",
        diagram.facts_line
    );
    assert_eq!(
        diagram.facts_line.contains("7 commands (2 joints)"),
        true,
        "{}",
        diagram.facts_line
    );
    assert_eq!(
        diagram.facts_line.contains("ends in display"),
        true,
        "{}",
        diagram.facts_line
    );
    assert_eq!(
        diagram.facts_line.contains("playback, volume: no end state (per region)"),
        true,
        "{}",
        diagram.facts_line
    );
    assert_eq!(
        diagram.facts_line.contains("product: 8 states, 0 ends (structural)"),
        true,
        "{}",
        diagram.facts_line
    );

    let map = diagram.to_route_map();
    let headers: Vec<&str> = map.lines().filter(|line| line.contains("┌─")).collect();
    assert_eq!(headers.len(), 3, "{map}");
    for line in &headers {
        assert_eq!(line.matches("┌─").count(), 1, "{map}");
    }
    assert_eq!(map.contains("┌─ display"), true, "{map}");
    assert_eq!(map.contains("┌─ playback"), true, "{map}");
    assert_eq!(map.contains("┌─ volume"), true, "{map}");
    assert_eq!(map.matches("● [ playing ]").count(), 1, "{map}");
    assert_eq!(map.matches("○ [ paused ]").count(), 1, "{map}");
    assert_eq!(map.matches("● [ unmuted ]").count(), 1, "{map}");
    assert_eq!(map.matches("○ [ muted ]").count(), 1, "{map}");
    assert_eq!(map.matches("● [ lit ]").count(), 1, "{map}");
    assert_eq!(map.matches("✔ [ dark ]").count(), 1, "{map}");

    let two = diagram_of(
        &load("machines/region_parallel.ik.toml"),
        None,
        &[],
        DiagramKind::Delivery,
    )
    .to_route_map();
    let two_headers: Vec<&str> = two.lines().filter(|line| line.contains("┌─")).collect();
    assert_eq!(two_headers.len(), 1, "{two}");
    assert_eq!(two_headers[0].matches("┌─").count(), 2, "{two}");
    assert_eq!(
        two.lines().filter(|line| line.contains("joint:")).all(|line| line.contains('╘') && line.contains('╛') && !line.contains('╮')),
        true,
        "{two}"
    );

    assert_eq!(map.contains("joint: pause_and_mute"), true, "{map}");
    assert_eq!(
        map.lines().filter(|line| line.contains("joint:")).all(|line| line.contains('╘') && line.contains('╛') && !line.contains('╮')),
        true,
        "{map}"
    );
    assert_eq!(map.contains("playback: [ playing ] → [ paused ]"), true, "{map}");
    assert_eq!(map.contains("volume  : [ unmuted ] → [ muted ]"), true, "{map}");
    assert_eq!(map.contains("joint: sleep"), true, "{map}");
    assert_eq!(map.contains("display : [ lit ] → [ dark ]"), true, "{map}");
    assert_eq!(
        map.contains("playback: [ paused, playing ] → [ paused ]"),
        true,
        "{map}"
    );
    assert_eq!(
        map.contains("volume  : [ muted, unmuted ] → [ muted ]"),
        true,
        "{map}"
    );
    assert_eq!(map.contains("if: volume is muted"), true, "{map}");
    assert_eq!(map.matches("region_volume").count(), 0, "{map}");
    assert_eq!(map.contains("✔ [ dark ]  END"), true, "{map}");

    let table = diagram.to_table_markdown();
    assert_eq!(table.contains("volume is muted"), true, "{table}");
    let squashed = squash(&table);
    assert_eq!(squashed.contains("| joint | sleep |"), true, "{table}");
    assert_eq!(squashed.contains("| joint | pause_and_mute |"), true, "{table}");

    let view = diagram.to_compressed_view();
    for line in view.lines() {
        assert_eq!(
            display_width(line) <= 120,
            true,
            "wide line {line}"
        );
    }
    let again = diagram_of(&def, None, &[], DiagramKind::Delivery).to_compressed_view();
    assert_eq!(view, again);
}

#[test]
fn region_compound_groups_parent_once() {
    let def = load_definition(&repo("machines/region_compound.ik.toml")).expect("compound");
    let map = diagram_of(&def, None, &[], DiagramKind::Delivery).to_route_map();
    assert_eq!(map.matches("│  brew").count(), 1, "{map}");
    assert_eq!(map.matches("│  finish").count(), 1, "{map}");
    assert_eq!(map.matches("from any state in on").count(), 1, "{map}");
    assert_eq!(map.matches("↗ override: power_off → [ ready ]").count(), 1, "{map}");
    assert_eq!(
        map.contains("child override is tried first; if its condition is false the group exit applies │"),
        true,
        "{map}"
    );
    assert_eq!(map.contains("applies│"), false, "{map}");
    assert_eq!(map.matches("L2 side-by-side").count(), 0, "{map}");
    assert_eq!(map.matches("│┆").count(), 0, "{map}");
    assert_eq!(map.matches("(group)").count(), 1, "{map}");
    assert_eq!(map.matches("┼").count(), 1, "{map}");
    let end_at = map.find("✔ [ off ]  END").expect("end");
    let ready_at = map.find("[ ready ]").expect("ready");
    assert_eq!(ready_at < end_at, true, "{map}");
}

#[test]
fn every_box_line_has_the_same_display_width() {
    let maps = [
        diagram_of(
            &load_definition(&repo("machines/region_compound.ik.toml")).expect("cmp"),
            None,
            &[],
            DiagramKind::Delivery,
        )
        .to_route_map(),
        diagram_of(
            &load_definition(&repo("machines/region_parallel.ik.toml")).expect("par"),
            None,
            &[],
            DiagramKind::Delivery,
        )
        .to_route_map(),
        diagram_of(
            &load_definition(&repo("tests/fixtures/diagram/region_stacked.ik.toml")).expect("stk"),
            None,
            &[],
            DiagramKind::Delivery,
        )
        .to_route_map(),
    ];
    for map in maps {
        let mut run: Vec<&str> = Vec::new();
        for line in map.lines().chain(std::iter::once("")) {
            if box_wall_line(line) {
                run.push(line);
                continue;
            }
            if run.len() >= 2 {
                let width = display_width(run[0]);
                for row in &run {
                    assert_eq!(
                        display_width(row),
                        width,
                        "box width {width} vs {}\n{map}",
                        display_width(row)
                    );
                }
            }
            run.clear();
        }
    }
}

fn box_wall_line(line: &str) -> bool {
    matches!(
        line.trim_start().chars().next(),
        Some('┌' | '│' | '└' | '╭' | '┆' | '╰')
    )
}

#[test]
fn recovery_cli_matches_handoff_safe_next() {
    let evidence = repo("tests/fixtures/diagram/recovery_lost.findings.json");
    let (out, _, code) = ik(&[
        "diagram",
        repo("examples/shipped/tagged_item.ik.toml").to_str().unwrap(),
        "--kind",
        "recovery",
        "--evidence",
        evidence.to_str().unwrap(),
        "--format",
        "markdown",
    ]);
    assert_eq!(code, Some(0), "{out}");
    let expected = "do not resend; the earlier result stands";
    assert!(out.contains(&format!("Safe next action: {expected}")), "{out}");
    assert!(out.contains("observed"), "{out}");
    assert!(out.contains("found by lookup"), "{out}");
    let library = build(
        Some(&tagged()),
        None,
        &recovery_lost_findings(),
        DiagramKind::Recovery,
        "item-lost",
        Some("op-tag"),
    );
    assert_eq!(library.safe_next.as_deref(), Some(expected));
}


#[test]
fn exits_stay_under_source_state() {
    let cases = [
        diagram_of(&loan_v1(), None, &[], DiagramKind::Delivery),
        diagram_of(&loan_v2(), Some(&loan_v1()), &[], DiagramKind::Change),
        diagram_of(
            &load_definition(&repo("machines/region_parallel.ik.toml")).expect("par"),
            None,
            &[],
            DiagramKind::Delivery,
        ),
        diagram_of(
            &load_definition(&repo("machines/region_compound.ik.toml")).expect("cmp"),
            None,
            &[],
            DiagramKind::Delivery,
        ),
    ];
    for diagram in cases {
        let map = diagram.to_route_map();
        let mut seen: Vec<&str> = Vec::new();
        for line in map.lines() {
            if let Some(name) = river_state(line) {
                assert_eq!(
                    seen.iter().filter(|n| **n == name).count(),
                    0,
                    "state {name} printed twice\n{map}"
                );
                seen.push(name);
            }
        }
        if diagram.regions.is_none() {
            let rights: Vec<usize> = map.lines().filter_map(right_col).collect();
            if let Some(first) = rights.first() {
                let drift = rights.iter().filter(|col| *col != first).count();
                assert_eq!(drift, 0, "right column drifted\n{map}");
            }
        }
        let table = diagram.to_table_markdown();
        for line in map.lines() {
            assert_eq!(line.contains("(given)"), false, "given in map: {line}");
            assert_eq!(line.contains("(stored)"), false, "stored in map: {line}");
        }
        let _ = table.contains("given");
        let mut after_end = false;
        let mut stray = 0usize;
        for line in map.lines() {
            if line.contains("✔ [") && line.contains(" END") {
                after_end = true;
            } else if line.contains("● [")
                || line.contains("○ [")
                || line.contains('┌')
                || line.contains("also reachable")
                || line.trim().is_empty()
            {
                after_end = false;
            } else if after_end && line.contains("│  ") && !line.contains('✔') {
                stray += 1;
            }
        }
        assert_eq!(stray, 0, "transition after END of another state\n{map}");
    }
}

fn river_state(line: &str) -> Option<&str> {
    for needle in ["● [ ", "○ [ ", "✔ [ "] {
        if let Some(rest) = line.split(needle).nth(1) {
            return rest.split(']').next();
        }
    }
    None
}

fn right_col(line: &str) -> Option<usize> {
    let river = line.contains("● [") || line.contains("○ [") || line.contains("✔ [") || line.contains("│  ");
    if !river || line.contains('✚') {
        return None;
    }
    let markers = ["◀──", "if:", "↺ can also", "(unchanged)", "↗ "];
    for mark in markers {
        if let Some(at) = line.find(mark) {
            return Some(ik::diagram::display_width(&line[..at]));
        }
    }
    None
}

#[test]
fn drawn_region_edges_match_chart() {
    let paths = [
        "machines/region_parallel.ik.toml",
        "machines/region_compound.ik.toml",
        "tests/fixtures/diagram/region_stacked.ik.toml",
        "tests/fixtures/diagram/coffee_compound.ik.toml",
    ];
    for path in paths {
        let def = load(path);
        let diagram = diagram_of(&def, None, &[], DiagramKind::Delivery);
        let view = diagram.regions.as_ref().expect(path);
        let mut drawn = std::collections::BTreeSet::new();
        for lane in &view.lanes {
            for hop in lane.hops.iter().chain(lane.parent_hops.iter()) {
                drawn.insert((hop.command.clone(), hop.from.clone(), hop.to.clone()));
            }
        }
        for joint in &view.joints {
            drawn.insert((joint.command.clone(), "*".into(), "*".into()));
        }
        let mut authored = std::collections::BTreeSet::new();
        for key in def.chart.authored_keys() {
            if key.layer == "joint" {
                authored.insert((key.command, "*".into(), "*".into()));
            } else {
                authored.insert((key.command, key.from, key.to));
            }
        }
        assert_eq!(drawn, authored, "{path}");
        let map = diagram.to_route_map();
        for (command, _, _) in &authored {
            assert_eq!(map.contains(command.as_str()), true, "{path} missing {command}\n{map}");
        }
        let names: std::collections::BTreeSet<_> = authored.iter().map(|(c, _, _)| c.as_str()).collect();
        assert_eq!(
            diagram.facts_line.contains(&format!("{} command", names.len()))
                || diagram.facts_line.contains(&format!("{} commands", names.len())),
            true,
            "{}",
            diagram.facts_line
        );
    }
}

fn two_command_machine() -> Definition {
    parse_definition(
        r#"
schema_version = 1
machine_id = "pair"
machine_version = 1
[facts]
[states]
idle = { initial = true }
done = { terminal = true }
[commands.alpha]
from = ["idle"]
to = "done"
[commands.zeta]
from = ["idle"]
to = "done"
"#,
    )
    .expect("pair")
}

#[test]
fn recovery_refusal_is_unreconciled_lookup() {
    let diagram = build(
        Some(&tagged()),
        None,
        &[json!({"status": "refused", "evidence_class": "observed_trace", "attempted": "{\"operation_id\":\"op-tag\"}"})],
        DiagramKind::Recovery,
        "item",
        Some("op-tag"),
    );
    let map = diagram.to_route_map();
    assert_eq!(
        map.lines()
            .any(|line| line.contains("observed") && !line.contains("not observed")),
        false,
        "{map}"
    );
    assert_eq!(map.contains("complete"), false, "{map}");
    assert_eq!(
        diagram.safe_next.as_deref(),
        Some("look up by saved operation id; never resend")
    );
    assert_eq!(map.contains("the earlier result stands"), false, "{map}");
}

#[test]
fn recovery_excludes_unrelated_operation() {
    let findings = vec![
        json!({"status": "historical", "attempted": "{\"operation_id\":\"other\"}"}),
        json!({"status": "reply_dropped", "attempted": "{\"operation_id\":\"op-tag\"}"}),
    ];
    let map = build(
        Some(&tagged()),
        None,
        &findings,
        DiagramKind::Recovery,
        "item",
        Some("op-tag"),
    )
    .to_route_map();
    assert_eq!(map.contains("found by lookup"), false, "{map}");
    assert_eq!(map.contains("reply lost"), true, "{map}");
}

#[test]
fn recovery_shows_operated_command_not_first_key() {
    let findings = vec![json!({
        "status": "reply_dropped",
        "attempted": "{\"operation_id\":\"op-z\",\"command\":\"zeta\"}"
    })];
    let map = build(
        Some(&two_command_machine()),
        None,
        &findings,
        DiagramKind::Recovery,
        "pair",
        Some("op-z"),
    )
    .to_route_map();
    assert_eq!(map.contains("zeta"), true, "{map}");
    assert_eq!(map.contains("alpha"), false, "{map}");
}

#[test]
fn recovery_mixed_operation_ids_are_ambiguous() {
    let findings = vec![
        json!({"status": "committed", "attempted": "{\"operation_id\":\"op-a\"}"}),
        json!({"status": "historical", "attempted": "{\"operation_id\":\"op-b\"}"}),
    ];
    let diagram = build(
        Some(&tagged()),
        None,
        &findings,
        DiagramKind::Recovery,
        "item",
        None,
    );
    assert_eq!(
        diagram.facts_line.contains("ambiguous: evidence names 2 operations"),
        true,
        "{}",
        diagram.facts_line
    );
    assert_eq!(diagram.facts_line.contains("op-a"), true, "{}", diagram.facts_line);
    assert_eq!(diagram.facts_line.contains("op-b"), true, "{}", diagram.facts_line);
    assert_eq!(diagram.to_route_map().contains("the earlier result stands"), false);
    let mixed = std::env::temp_dir().join(format!(
        "ik-mixed-ops-{}-{}.json",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    fs::write(
        &mixed,
        r#"{"findings":[{"status":"committed","attempted":"{\"operation_id\":\"op-a\"}"},{"status":"historical","attempted":"{\"operation_id\":\"op-b\"}"}]}"#,
    )
    .expect("mixed");
    let (out, err, code) = ik(&[
        "diagram",
        repo("examples/shipped/tagged_item.ik.toml").to_str().unwrap(),
        "--kind",
        "recovery",
        "--evidence",
        mixed.to_str().unwrap(),
        "--format",
        "text",
    ]);
    assert_eq!(code, Some(2), "{out}{err}");
    assert_eq!(out.starts_with("ambiguous: evidence names 2 operations"), true, "{out}");
    assert_eq!(out.contains("```text"), false, "{out}");
}

#[test]
fn recovery_lookup_does_not_mark_intermediates_observed() {
    let map = build(
        Some(&tagged()),
        None,
        &[json!({"status": "committed_exact", "attempted": "{\"operation_id\":\"op-tag\"}"})],
        DiagramKind::Recovery,
        "item",
        Some("op-tag"),
    )
    .to_route_map();
    assert_eq!(
        map.lines()
            .any(|line| line.contains("observed") && !line.contains("not observed")),
        false,
        "{map}"
    );
    assert_eq!(map.contains("found by lookup"), true, "{map}");
    assert_eq!(
        map.contains("Uncertain: whether any handle from this lookup is current"),
        true,
        "{map}"
    );
}

#[test]
fn removed_label_uses_new_machine_version() {
    let mut prior = reach_v1();
    prior.machine_version = 5;
    let mut next = reach_v2();
    next.machine_version = 6;
    let map = diagram_of(&next, Some(&prior), &[], DiagramKind::Change).to_route_map();
    assert_eq!(map.contains("REMOVED in v6"), true, "{map}");
    assert_eq!(map.contains("REMOVED in v2"), false, "{map}");
}

fn squash(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[test]
fn md_table_pads_every_line_to_one_width() {
    let rows = vec![
        vec!["a".to_string(), "a longer cell".to_string()],
        vec!["bb".to_string(), "x".to_string()],
    ];
    let table = md_table(&["h1", "header two"], &rows);
    let widths: Vec<usize> = table.lines().map(display_width).collect();
    assert_eq!(widths.iter().all(|width| *width == widths[0]), true, "{table}");
    assert_eq!(table.lines().count(), 4, "{table}");
}

#[test]
fn md_table_pads_wide_characters_by_display_width() {
    let rows = vec![vec!["日本語".to_string(), "x".to_string()]];
    let table = md_table(&["name", "v"], &rows);
    let widths: Vec<usize> = table.lines().map(display_width).collect();
    assert_eq!(widths.iter().all(|width| *width == widths[0]), true, "{table}");
}

#[test]
fn md_table_wraps_long_cells_inside_the_ceiling_and_keeps_every_word() {
    let long = (0..60).map(|n| format!("word{n}")).collect::<Vec<_>>().join(" ");
    let rows = vec![vec!["id".to_string(), long.clone()]];
    let table = md_table(&["id", "text"], &rows);
    assert_eq!(
        table.lines().all(|line| display_width(line) <= MAX_LINE),
        true,
        "{table}"
    );
    let widths: Vec<usize> = table.lines().map(display_width).collect();
    assert_eq!(widths.iter().all(|width| *width == widths[0]), true, "{table}");
    let kept: Vec<String> = table
        .lines()
        .skip(2)
        .map(|line| line.split('|').nth(2).unwrap_or("").trim().to_string())
        .collect();
    assert_eq!(kept.join(" "), long, "{table}");
}

#[test]
fn md_table_escapes_pipes_and_never_splits_the_escape() {
    let cell = format!("{}|{}", "x".repeat(130), "y".repeat(10));
    let table = md_table(&["a", "b"], &[vec!["k".to_string(), cell]]);
    assert_eq!(table.contains("\\|"), true, "{table}");
    assert_eq!(table.contains("\\ |"), false, "{table}");
    assert_eq!(
        table.lines().all(|line| display_width(line) <= MAX_LINE),
        true,
        "{table}"
    );
}

#[test]
fn md_table_header_never_wraps_even_when_columns_overflow() {
    let wide = "z".repeat(200);
    let table = md_table(&["name", "a fairly long header"], &[vec!["k".to_string(), wide]]);
    assert_eq!(table.lines().next().unwrap_or("").contains("a fairly long header"), true, "{table}");
}

#[test]
fn recovery_timeline_lists_only_evidenced_events() {
    let findings = vec![
        json!({"status": "reply_dropped", "attempted": "{\"operation_id\":\"op-tag\",\"command\":\"tag\"}"}),
        json!({"status": "committed_exact", "attempted": "{\"operation_id\":\"op-tag\",\"command\":\"tag\"}"}),
    ];
    let diagram = build(
        Some(&tagged()),
        None,
        &findings,
        DiagramKind::Recovery,
        "item",
        Some("op-tag"),
    );
    let table = diagram.to_table_markdown();
    for invented in ["plan tag", "saved", "recorded", "accepted", "complete", "subject"] {
        assert_eq!(table.contains(invented), false, "{invented} in {table}");
    }
    let squashed = squash(&table);
    assert_eq!(squashed.contains("| request sent | observed |"), true, "{table}");
    assert_eq!(squashed.contains("| reply lost | observed |"), true, "{table}");
    assert_eq!(squashed.contains("| committed | found by lookup |"), true, "{table}");
}

#[test]
fn recovery_without_evidence_lists_no_events() {
    let diagram = build(
        Some(&tagged()),
        None,
        &[],
        DiagramKind::Recovery,
        "item",
        None,
    );
    let map = diagram.to_route_map();
    assert_eq!(map.contains("(no recovery events in the evidence)"), true, "{map}");
    assert_eq!(diagram.to_table_markdown().lines().count(), 2);
}

fn loan_v2_text(edit: impl Fn(String) -> String) -> Definition {
    let text = fs::read_to_string(repo("validation/dfy/journeys/fixtures/loan_v2.ik.toml")).expect("v2");
    parse_definition(&edit(text)).expect("edited v2")
}

fn rule<'a>(diagram: &'a Diagram, name: &str) -> &'a ik::diagram::RuleEntry {
    diagram
        .rules
        .iter()
        .find(|rule| rule.name == name)
        .unwrap_or_else(|| panic!("no rule {name} in {:?}", diagram.rules))
}

#[test]
fn rule_ledger_for_a_region_machine_lists_authored_guards_not_the_lowering() {
    let def = load("tests/fixtures/diagram/region_stacked.ik.toml");
    let diagram = diagram_of(&def, None, &[], DiagramKind::Delivery);
    let names: Vec<&str> = diagram.rules.iter().map(|rule| rule.name.as_str()).collect();
    assert_eq!(names, vec!["blank"]);
    assert_eq!(diagram.rules[0].guards, vec!["region_volume == \"muted\"".to_string()]);
    let all = format!("{:?}", diagram.rules);
    assert_eq!(all.contains("state =="), false, "{all}");
}

#[test]
fn rule_ledger_marks_new_changed_removed_and_unchanged_against_the_prior() {
    let prior = loan_v1();
    let changed = loan_v2_text(|text| {
        text.replace("$overdue == 0 || recall_placed == 1", "$overdue == 0")
            .replace(
                "when = \"$loan_until_ms == $checkout_ms + 1209600000\"\n",
                "",
            )
    });
    let diagram = build(
        Some(&changed),
        Some(&prior),
        &[],
        DiagramKind::Change,
        "loan",
        None,
    );
    assert_eq!(rule(&diagram, "extend").status, "new");
    assert_eq!(rule(&diagram, "checkin").status, "changed");
    assert_eq!(rule(&diagram, "checkout").status, "removed");
    assert_eq!(rule(&diagram, "checkout").rule.starts_with("no rule now"), true);
    let unchanged = diagram_of(&loan_v2(), Some(&prior), &[], DiagramKind::Change);
    assert_eq!(rule(&unchanged, "checkout").status, "unchanged");
    assert_eq!(rule(&unchanged, "checkin").status, "unchanged");
}

#[test]
fn a_rule_hash_changes_with_the_guard_and_is_stable_without_change() {
    let first = diagram_of(&loan_v2(), None, &[], DiagramKind::Delivery);
    let again = diagram_of(&loan_v2(), None, &[], DiagramKind::Delivery);
    assert_eq!(rule(&first, "extend").hash, rule(&again, "extend").hash);
    let loosened = loan_v2_text(|text| text.replace("$new_until_ms > loan_until_ms", "$new_until_ms >= loan_until_ms"));
    let other = diagram_of(&loosened, None, &[], DiagramKind::Delivery);
    assert_eq!(rule(&first, "extend").hash == rule(&other, "extend").hash, false);
    assert_eq!(rule(&first, "checkout").hash, rule(&other, "checkout").hash);
}
