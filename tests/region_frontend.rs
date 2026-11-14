#[path = "common/mod.rs"]
mod common;

use common::*;
use ik::declarative::compile::compile;
use ik::declarative::explore::explore_with_invariant_witnesses;
use ik::declarative::expr::{self, Cmp, CmpOp, Expr, Operand};
use ik::declarative::region::{parse_source, parse_source_with_limits, RegionLimits, RegionMap};
use ik::declarative::{parse_definition, Bounds, CaseCorpus, CompiledMachine, Definition};
use ik::diagnostic::DiagnosticCode;
use ik::effect::EffectStatus;
use ik::machine::{Decision, Machine};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

const COMPOUND: &str = include_str!("../machines/region_compound.ik.toml");
const PARALLEL: &str = include_str!("../machines/region_parallel.ik.toml");

fn model(text: &str) -> (CompiledMachine, RegionMap) {
    let source = parse_source(text).unwrap_or_else(|e| panic!("{e:?}"));
    (
        compile(source.definition).unwrap_or_else(|e| panic!("{e:?}")),
        source.regions.unwrap(),
    )
}

fn event(machine: &CompiledMachine, state: &Value, command: Value) -> ik::machine::DomainEvent {
    match machine
        .decide(state, None, &EffectStatus::None, &command)
        .expect("command decision")
    {
        Decision::Emit { event, .. } => event,
        Decision::Noop { .. } => panic!("expected transition"),
    }
}

fn step(machine: &CompiledMachine, state: &Value, command: Value) -> Value {
    machine
        .apply(state, &event(machine, state, command))
        .expect("safe transition")
        .state
}

fn decoded(map: &RegionMap, state: &Value, expected: &[(&str, &str)]) {
    let expected: BTreeMap<String, String> = expected
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    assert_eq!(map.decode(state).expect("decode flat state"), &expected);
}

#[test]
fn local_commands_preserve_the_sibling_and_joint_updates_both() {
    let (machine, regions) = model(PARALLEL);
    let initial = machine.initial_state();
    decoded(
        &regions,
        &initial,
        &[("playback", "playing"), ("volume", "unmuted")],
    );
    let paused = step(&machine, &initial, json!({"name":"pause"}));
    decoded(
        &regions,
        &paused,
        &[("playback", "paused"), ("volume", "unmuted")],
    );
    let muted = step(&machine, &paused, json!({"name":"mute"}));
    decoded(
        &regions,
        &muted,
        &[("playback", "paused"), ("volume", "muted")],
    );
    let unmuted = step(&machine, &muted, json!({"name":"unmute"}));
    decoded(
        &regions,
        &unmuted,
        &[("playback", "paused"), ("volume", "unmuted")],
    );
    let joint = step(&machine, &initial, json!({"name":"pause_and_mute"}));
    decoded(
        &regions,
        &joint,
        &[("playback", "paused"), ("volume", "muted")],
    );
    assert_eq!(
        [&initial, &paused, &muted, &unmuted, &joint],
        [
            &json!({"state":"playing__unmuted","region_playback":"playing","region_volume":"unmuted"}),
            &json!({"state":"paused__unmuted","region_playback":"paused","region_volume":"unmuted"}),
            &json!({"state":"paused__muted","region_playback":"paused","region_volume":"muted"}),
            &json!({"state":"paused__unmuted","region_playback":"paused","region_volume":"unmuted"}),
            &json!({"state":"paused__muted","region_playback":"paused","region_volume":"muted"}),
        ],
        "local preservation and joint projection are exact, not merely decodable"
    );
}

#[test]
fn guarded_child_wins_when_enabled_and_parent_falls_back_when_disabled() {
    let (machine, regions) = model(COMPOUND);
    let initial = machine.initial_state();
    let off = step(
        &machine,
        &initial,
        json!({"name":"power_off", "args":{"override_enabled":false}}),
    );
    decoded(&regions, &off, &[("power", "off")]);
    let ready = step(
        &machine,
        &initial,
        json!({"name":"power_off", "args":{"override_enabled":true}}),
    );
    decoded(&regions, &ready, &[("power", "ready")]);
    let brewing = step(&machine, &initial, json!({"name":"brew"}));
    decoded(&regions, &brewing, &[("power", "brewing")]);
    let finished = step(&machine, &brewing, json!({"name":"finish"}));
    decoded(&regions, &finished, &[("power", "ready")]);
    for state in [&brewing, &finished] {
        let result = step(
            &machine,
            state,
            json!({"name":"power_off", "args":{"override_enabled":true}}),
        );
        decoded(&regions, &result, &[("power", "off")]);
        assert_eq!(result, json!({"state":"off","region_power":"off"}));
    }
    assert_eq!(
        [off, ready, brewing, finished],
        [
            json!({"state":"off","region_power":"off"}),
            json!({"state":"ready","region_power":"ready"}),
            json!({"state":"brewing","region_power":"brewing"}),
            json!({"state":"ready","region_power":"ready"}),
        ]
    );
}

#[test]
fn invariant_refusal_projects_and_replays_without_committing_the_unsafe_target() {
    let (machine, regions) = model(PARALLEL);
    let corpus = CaseCorpus::new();
    let (run, witnesses) = explore_with_invariant_witnesses(
        &machine,
        &corpus,
        Bounds {
            max_nodes: 128,
            max_depth: 8,
        },
    );
    assert!(run.complete);
    assert!(!run
        .coverage
        .executably_reached_states
        .contains(&"playing__muted".to_string()));
    let witness = witnesses
        .iter()
        .find(|w| w.diagnostic.rule.as_deref() == Some("no_playing_muted"))
        .expect("cross-region invariant refusal");
    let path = regions
        .project_path(&machine, &witness.steps, &corpus)
        .expect("project authoritative replay");
    let refused = path.last().expect("refused step");
    assert!(!refused.committed);
    assert_eq!(refused.diagnostic.as_ref(), Some(&witness.diagnostic));
    assert_eq!(
        refused.target,
        BTreeMap::from([
            ("playback".to_string(), "playing".to_string()),
            ("volume".to_string(), "muted".to_string())
        ])
    );
    let mut state = machine.initial_state();
    for (index, command) in witness.steps.iter().enumerate() {
        let candidate = event(&machine, &state, json!({"name":command.command}));
        assert_eq!(
            candidate.payload["outcome"].as_u64().map(|v| v as usize),
            command.outcome_index
        );
        match machine.apply(&state, &candidate) {
            Ok(next) => {
                assert!(index + 1 < witness.steps.len());
                state = next.state;
            }
            Err(error) => {
                assert_eq!(index + 1, witness.steps.len());
                assert_eq!(error, witness.diagnostic);
            }
        }
    }
    assert_eq!(regions.decode(&state).unwrap(), &refused.before);
    let mut continued = witness.steps.clone();
    continued.push(witness.steps.last().unwrap().clone());
    assert!(regions.project_path(&machine, &continued, &corpus).is_err());
}

fn refused(text: &str, description: &str) {
    assert!(parse_source(text).is_err(), "must refuse {description}");
}

fn identity_cli(text: &str) -> std::process::Output {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("authored.ik.toml");
    std::fs::write(&source, text).unwrap();
    std::process::Command::new(env!("CARGO_BIN_EXE_ik"))
        .arg("identity")
        .arg(source)
        .output()
        .unwrap()
}

#[test]
fn malformed_authoring_is_refused_before_runtime() {
    // Configuration oracle: docs/region-lowering.md. Each case starts from a
    // valid source and changes one independently forbidden declaration. Match
    // the causal diagnostic, not an arbitrary earlier parse failure.
    assert!(parse_source(PARALLEL).is_ok());
    assert!(parse_source(COMPOUND).is_ok());
    let mut cases = Vec::new();
    for (old, new, cause) in [
        ("from = [\"playing\"]", "from = [\"ghost\"]", "unknown source state `ghost`"),
        ("to = \"paused\"", "to = \"ghost\"", "targets unknown state `ghost`"),
        ("volume = [\"unmuted\"]", "ghost = [\"unmuted\"]", "references unknown region `ghost`"),
        ("volume = \"muted\"", "ghost = \"muted\"", "targets unknown region `ghost`"),
        ("volume = [\"unmuted\"]", "volume = [\"ghost\"]", "unknown sources in `volume`"),
        ("volume = \"muted\"", "volume = \"ghost\"", "targets unknown state `volume.ghost`"),
        ("to = \"paused\"", "to = \"paused\"\nthen = { region_volume = '\"muted\"' }", "cannot assign generated `region_` facts"),
        ("to = \"paused\"", "to = \"paused\"\nwhen = 'state == \"playing\"'", "not ambiguous flat `state`"),
        ("to = \"paused\"", "to = \"paused\"\nwhen = 'region_playback == \"ghost\"'", "`region_playback` references unknown state `ghost`"),
        ("to = \"paused\"", "to = \"paused\"\nwhen = 'region_playback =='", "expression `region_playback =="),
        ("to = \"paused\"", "to = \"paused\"\nextra = true", "unknown field `extra`"),
        ("machine_version = 1", "machine_version = 1\nextra = true", "unknown field `extra`"),
        ("[regions.playback.states]", "[regions.playback]\nextra = true\n[regions.playback.states]", "unknown field `extra`"),
        ("paused = {}", "paused = { extra = true }", "unknown field `extra`"),
        ("[commands.pause_and_mute]", "[commands.pause_and_mute]\nextra = true", "unknown field `extra`"),
        ("to = \"paused\"", "to = \"paused\"\noutcomes = [{ to = \"paused\" }]", "must not combine `to`/`when` with `outcomes`"),
        ("to = \"paused\"", "outcomes = []", "empty `outcomes` list"),
        ("to = \"paused\"", "outcomes = [{ to = \"paused\" }, { to = \"playing\" }]", "later outcomes are unreachable"),
        ("from = { playback = [\"playing\"], volume = [\"unmuted\"] }", "from = {}", "requires a nonempty region source map"),
        ("volume = [\"unmuted\"]", "volume = []", "empty or unknown sources in `volume`"),
        ("to = { playback = \"paused\", volume = \"muted\" }", "to = {}", "requires a nonempty destination map"),
        ("to = { playback = \"paused\", volume = \"muted\" }", "outcomes = []", "must declare either to/when/then or nonempty outcomes"),
        ("to = { playback = \"paused\", volume = \"muted\" }", "to = { playback = \"paused\", volume = \"muted\" }\noutcomes = [{ to = { playback = \"paused\" } }]", "must declare either to/when/then or nonempty outcomes"),
        ("to = { playback = \"paused\", volume = \"muted\" }", "outcomes = [{ to = { playback = \"paused\" } }, { to = { volume = \"muted\" } }]", "unreachable outcomes after unconditional outcome"),
        ("to = { playback = \"paused\", volume = \"muted\" }", "to = { playback = \"paused\", volume = \"muted\" }\nthen = { region_playback = '\"paused\"' }", "cannot assign generated `region_` facts"),
        ("[regions.volume.commands.mute]", "[regions.volume.commands.pause]", "command `pause` has multiple region/joint owners"),
        ("[commands.pause_and_mute]", "[commands.pause]", "command `pause` has multiple region/joint owners"),
    ] {
        assert!(PARALLEL.contains(old), "fixture mutation anchor: {old}");
        cases.push((PARALLEL.replacen(old, new, 1), cause.to_string()));
    }
    for (old, new, cause) in [
        (
            "name = \"on\"",
            "name = \"grinding\"",
            "parent must be distinct and have children",
        ),
        (
            "name = \"on\"",
            "name = \"on\"\nextra = true",
            "unknown field `extra`",
        ),
        (
            "children = [\"grinding\", \"brewing\", \"ready\"]",
            "children = []",
            "parent must be distinct and have children",
        ),
        (
            "children = [\"grinding\", \"brewing\", \"ready\"]",
            "children = [\"grinding\", \"grinding\"]",
            "parent has duplicate or unknown children",
        ),
        (
            "children = [\"grinding\", \"brewing\", \"ready\"]",
            "children = [\"ghost\"]",
            "parent has duplicate or unknown children",
        ),
        (
            "from = [\"on\"]",
            "from = [\"grinding\"]",
            "parent command `power_off` must use from",
        ),
        (
            "grinding = { initial = true }",
            "grinding = {}",
            "requires exactly one initial state (found 0)",
        ),
        (
            "brewing = {}",
            "brewing = { initial = true }",
            "requires exactly one initial state (found 2)",
        ),
        (
            "grinding = { initial = true }",
            "grinding = { initial = true, terminal = true }",
            "initial state must not be terminal",
        ),
        (
            "from = [\"grinding\"]",
            "from = [\"off\"]",
            "lists terminal state `off` as a source",
        ),
    ] {
        assert!(COMPOUND.contains(old), "fixture mutation anchor: {old}");
        cases.push((COMPOUND.replacen(old, new, 1), cause.to_string()));
    }
    cases.push((
        format!("{PARALLEL}\n[facts.region_custom]\ntype = 'bool'\npolicy = 'replace'\ninitial = false\n"),
        "fact and invariant names beginning `region_` are reserved".into(),
    ));
    cases.push((
        format!("{PARALLEL}\n[invariants.region_custom]\nassert = 'true'\n"),
        "fact and invariant names beginning `region_` are reserved".into(),
    ));
    cases.push((
        "schema_version = 2\nmachine_id = 'empty'\nmachine_version = 1\nregions = {}\n".into(),
        "at least one region is required".into(),
    ));

    // Deterministic generative domain: seven forbidden identifier classes at
    // each of three name positions, and sixteen fresh unknown leaves at each
    // of local source/target, joint source/target, and region-expression sites.
    for name in [
        "",
        "0bad",
        "has-dash",
        "two__parts",
        "é",
        "with space",
        "a.b",
    ] {
        let quoted = serde_json::to_string(name).unwrap();
        cases.push((
            format!("schema_version = 2\nmachine_id = 'names'\nmachine_version = 1\n[regions.{quoted}.states]\ns0 = {{ initial = true }}\n"),
            format!("region `{name}` must be an ASCII identifier"),
        ));
        cases.push((
            format!("schema_version = 2\nmachine_id = 'names'\nmachine_version = 1\n[regions.valid.states]\n{quoted} = {{ initial = true }}\n"),
            format!("state `{name}` must be an ASCII identifier"),
        ));
        cases.push((
            COMPOUND.replace("name = \"on\"", &format!("name = {quoted}")),
            format!("parent `{name}` must be an ASCII identifier"),
        ));
    }
    for index in 0..16 {
        let unknown = format!("missing_{index}");
        for (old, new, cause) in [
            (
                "from = [\"playing\"]",
                format!("from = [\"{unknown}\"]"),
                format!("unknown source state `{unknown}`"),
            ),
            (
                "to = \"paused\"",
                format!("to = \"{unknown}\""),
                format!("targets unknown state `{unknown}`"),
            ),
            (
                "volume = [\"unmuted\"]",
                format!("volume = [\"{unknown}\"]"),
                "unknown sources in `volume`".into(),
            ),
            (
                "volume = \"muted\"",
                format!("volume = \"{unknown}\""),
                format!("targets unknown state `volume.{unknown}`"),
            ),
            (
                "to = \"paused\"",
                format!("to = \"paused\"\nwhen = 'region_playback == \"{unknown}\"'"),
                format!("`region_playback` references unknown state `{unknown}`"),
            ),
        ] {
            cases.push((PARALLEL.replacen(old, &new, 1), cause));
        }
    }
    // Structural flat-state prohibition at both operand positions, below a
    // boolean node and below arithmetic; cross each shape with all five
    // authored expression locations, not the generated schema-1 selectors.
    for expression in [
        "state == \"playing\"",
        "\"playing\" == state",
        "(region_volume == \"unmuted\") && (state == \"playing\")",
        "state + 1 == 1",
    ] {
        for (old, replacement) in [
            (
                "to = \"paused\"",
                format!("to = \"paused\"\nwhen = '{expression}'"),
            ),
            (
                "to = \"paused\"",
                format!("outcomes = [{{ when = '{expression}', to = \"paused\" }}]"),
            ),
            (
                "to = { playback = \"paused\", volume = \"muted\" }",
                format!(
                    "to = {{ playback = \"paused\", volume = \"muted\" }}\nwhen = '{expression}'"
                ),
            ),
            (
                "assert = 'region_playback != \"playing\" || region_volume != \"muted\"'",
                format!("assert = '{expression}'"),
            ),
            (
                "[invariants.no_playing_muted]",
                format!("[invariants.no_playing_muted]\nwhen = '{expression}'"),
            ),
        ] {
            cases.push((
                PARALLEL.replacen(old, &replacement, 1),
                "not ambiguous flat `state`".into(),
            ));
        }
    }
    for (text, cause) in cases {
        let errors = parse_source(&text)
            .err()
            .expect("invalid authoring must refuse");
        assert!(
            errors.iter().any(|error| error.contains(&cause)),
            "must refuse for {cause:?}, not an unrelated cause: {errors:?}"
        );
        let output = identity_cli(&text);
        assert_eq!(
            output.status.code(),
            Some(2),
            "CLI accepted forbidden authoring: {cause}"
        );
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(
            stderr.contains(&cause),
            "CLI must refuse for {cause:?}, not another failure: {stderr}"
        );
    }
}

#[test]
fn child_parent_effect_declarations_must_match() {
    let effect = "effect = { kind = 'notify', id = '\"one\"', payload_digest = '\"payload\"', idempotency_key = '\"key\"' }\n";
    let matching = COMPOUND.replace(
        "args = { override_enabled = \"bool\" }\n",
        &format!("args = {{ override_enabled = \"bool\" }}\n{effect}"),
    );
    let (matching_machine, matching_regions) = model(&matching);
    let matching_result = step(
        &matching_machine,
        &matching_machine.initial_state(),
        json!({"name":"power_off", "args":{"override_enabled":false}}),
    );
    decoded(&matching_regions, &matching_result, &[("power", "off")]);
    let mismatched = matching.replacen("id = '\"one\"'", "id = '\"two\"'", 1);
    refused(&mismatched, "mismatched child/parent effects");
    assert_eq!(matching_result, json!({"state":"off","region_power":"off"}));
    assert!(parse_source(&mismatched)
        .err()
        .unwrap()
        .iter()
        .any(|error| error.contains(
            "child and parent must have exactly identical args and effect declarations"
        )));
    let mismatched_args = COMPOUND.replacen(
        "override_enabled = \"bool\"",
        "override_enabled = \"string\"",
        1,
    );
    refused(&mismatched_args, "mismatched child/parent argument types");
    assert!(parse_source(&mismatched_args)
        .err()
        .unwrap()
        .iter()
        .any(|error| error.contains(
            "child and parent must have exactly identical args and effect declarations"
        )));
}

#[test]
fn generated_projection_invariant_refuses_an_inconsistent_poststate() {
    let source = parse_source(PARALLEL).unwrap();
    let mut definition = source.definition;
    let outcomes = definition
        .commands
        .get_mut("pause")
        .unwrap()
        .outcomes
        .as_mut()
        .unwrap();
    for outcome in outcomes {
        outcome
            .then
            .as_mut()
            .unwrap()
            .insert("region_playback".into(), "\"playing\"".into());
    }
    let machine = compile(definition).unwrap();
    let initial = machine.initial_state();
    let error = machine
        .apply(
            &initial,
            &event(&machine, &initial, json!({"name":"pause"})),
        )
        .expect_err("projection disagreement must not commit");
    assert_eq!(error.code, DiagnosticCode::IkInvariantFailed);
    assert_eq!(
        error.rule.as_deref(),
        Some("region_projection_paused__unmuted")
    );
}

fn product_source(left: usize, right: usize) -> String {
    let mut text = "schema_version = 2\nmachine_id = 'product'\nmachine_version = 1\n".to_string();
    for (region, size) in [("left", left), ("right", right)] {
        text.push_str(&format!("[regions.{region}.states]\n"));
        for index in 0..size {
            text.push_str(&format!("s{index} = {{ initial = {} }}\n", index == 0));
        }
    }
    text
}

#[test]
fn product_bound_accepts_64_and_refuses_65() {
    let (machine, regions) = model(&product_source(8, 8));
    decoded(
        &regions,
        &machine.initial_state(),
        &[("left", "s0"), ("right", "s0")],
    );
    assert_eq!(
        machine
            .definition()
            .states
            .keys()
            .cloned()
            .collect::<Vec<_>>(),
        vec!["s0__s0"]
    );
    let output = identity_cli(&product_source(8, 8));
    assert_eq!(
        output.status.code(),
        Some(0),
        "64-state CLI boundary must accept"
    );
    let identity: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(identity["machine_id"], "product");
    assert_eq!(identity["version"], 1);
    let errors = parse_source(&product_source(5, 13)).err().unwrap();
    assert!(
        errors
            .iter()
            .any(|error| error.contains("product exceeds 64 states (left=5, right=13)")),
        "{errors:?}"
    );
    let output = identity_cli(&product_source(5, 13));
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8(output.stderr)
        .unwrap()
        .contains("product exceeds 64 states (left=5, right=13)"));

    // Only 256 authored leaves, but 2^128 theoretical configurations. The bound
    // must win before constructing that product (or validating the poisoned
    // final identifier). No timing-sensitive threshold or allocation hook.
    let mut enormous =
        "schema_version = 2\nmachine_id = 'enormous'\nmachine_version = 1\n".to_string();
    for index in 0..128 {
        enormous.push_str(&format!(
            "[regions.r{index:03}.states]\ns0 = {{ initial = true }}\n"
        ));
        enormous.push_str(if index == 127 {
            "'bad__leaf' = {}\n"
        } else {
            "s1 = {}\n"
        });
    }
    let errors = parse_source(&enormous)
        .err()
        .expect("huge product refuses before expansion");
    assert!(
        errors
            .iter()
            .any(|error| error.contains("product exceeds 64 states")
                && error.contains("r000=2")
                && error.contains("r127=2")),
        "{errors:?}"
    );
    assert!(
        !errors
            .iter()
            .any(|error| error.contains("ASCII identifier")),
        "bound must precede per-leaf validation: {errors:?}"
    );
    let output = identity_cli(&enormous);
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("product exceeds 64 states") && stderr.contains("r127=2"),
        "{stderr}"
    );
}

#[test]
fn schema2_identity_ignores_comments_and_table_order() {
    let reordered = PARALLEL
        .replace(
            "playing = { initial = true }\npaused = {}",
            "paused = {}\n# reordered declarations\nplaying = { initial = true }",
        )
        .replace(
            "unmuted = { initial = true }\nmuted = {}",
            "muted = {}\nunmuted = { initial = true }",
        );
    let (original, _) = model(PARALLEL);
    let (other, _) = model(&format!("# author comment\n{reordered}"));
    assert_eq!(original.identity(), other.identity());
    let a = step(
        &original,
        &original.initial_state(),
        json!({"name":"pause_and_mute"}),
    );
    let b = step(
        &other,
        &other.initial_state(),
        json!({"name":"pause_and_mute"}),
    );
    assert_eq!(a, b);
}

#[test]
fn schema1_direct_deserialization_and_frontend_have_identical_identity() {
    let text = include_str!("../machines/invariant_probe.ik.toml");
    let direct: Definition = toml::from_str(text).unwrap();
    let original = compile(direct).unwrap();
    let frontend = compile(parse_definition(text).unwrap()).unwrap();
    assert_eq!(original.identity(), frontend.identity());
    assert_eq!(original.initial_state(), frontend.initial_state());
}

#[test]
fn cli_create_command_and_replay_use_the_same_region_machine() {
    let dir = tempfile::tempdir().unwrap();
    let store = dir.path().join("regions.db");
    let definition = machine_def("region_parallel");
    let machine = load_machine("region_parallel");
    let request = |op: &str| json!({"protocol":{"major":1,"minor":0},"runtime_version":ik::protocol::RUNTIME_VERSION,"request_id":op,"op":op,"aggregate_id":"regions","machine":machine.identity()});
    let created = run_cli_json(&store, Some(&definition), &request("create"));
    assert_eq!(created["ok"], true, "{created}");
    let mut command = request("command");
    command["capability"] = created["result"]["capability"].clone();
    command["command"] = json!({"name":"pause_and_mute"});
    let committed = run_cli_json(&store, Some(&definition), &command);
    assert_eq!(committed["ok"], true, "{committed}");
    let (_, regions) = model(PARALLEL);
    decoded(
        &regions,
        &committed["result"]["state"],
        &[("playback", "paused"), ("volume", "muted")],
    );
    let replayed = run_cli_json(&store, Some(&definition), &request("replay"));
    assert_eq!(replayed["ok"], true, "{replayed}");
    assert_eq!(
        replayed["result"]["aggregates"][0]["state"],
        committed["result"]["state"]
    );
}

#[test]
fn initially_unsafe_region_configuration_is_refused() {
    let unsafe_initial = PARALLEL.replace(
        "unmuted = { initial = true }\nmuted = {}",
        "unmuted = {}\nmuted = { initial = true }",
    );
    refused(
        &unsafe_initial,
        "initial playing plus muted invariant violation",
    );
    let errors = parse_source(&unsafe_initial).err().unwrap();
    assert!(
        errors.iter().any(|error| error
            .contains("initial region configuration violates invariant")
            && error.contains("no_playing_muted")),
        "{errors:?}"
    );
}

#[test]
fn corrupted_projection_facts_refuse_decision_instead_of_being_healed() {
    let (machine, _) = model(PARALLEL);
    let mut corrupted = machine.initial_state();
    corrupted["region_volume"] = json!("muted");
    assert!(machine
        .decide(
            &corrupted,
            None,
            &EffectStatus::None,
            &json!({"name":"pause"})
        )
        .is_err());
}

#[test]
fn reusable_check_reports_invariant_refusal_and_validates_witness_classification() {
    let source = parse_source(PARALLEL).unwrap();
    let corpus = CaseCorpus::new();
    let result = ik::declarative::region::check_source(
        &source,
        None,
        &corpus,
        &[],
        Bounds {
            max_nodes: 128,
            max_depth: 8,
        },
    )
    .unwrap();
    assert!(result.text.contains("REASONS"), "{}", result.text);
    for reason in &result.flat.judgment.reasons {
        assert!(
            result.text.contains(reason),
            "missing rendered reason: {reason}\n{}",
            result.text
        );
    }
    assert_eq!(result.verdict, "REFUSED");
    let machine = compile(source.definition).unwrap();
    let regions = source.regions.unwrap();
    let witness = result
        .invariant_refusals
        .iter()
        .find(|w| w.diagnostic.rule.as_deref() == Some("no_playing_muted"))
        .unwrap();
    let path = regions
        .project_invariant_witness(&machine, witness, &corpus)
        .unwrap();
    let last = path.last().unwrap();
    assert!(!last.committed);
    assert!(
        [
            BTreeMap::from([
                ("playback".into(), "playing".into()),
                ("volume".into(), "unmuted".into())
            ]),
            BTreeMap::from([
                ("playback".into(), "paused".into()),
                ("volume".into(), "muted".into())
            ]),
        ]
        .contains(&last.before),
        "a refusal starts at one of the two independently safe predecessors"
    );
    assert_eq!(
        last.target,
        BTreeMap::from([
            ("playback".into(), "playing".into()),
            ("volume".into(), "muted".into())
        ])
    );
    assert_eq!(
        last.diagnostic.as_ref().unwrap().code,
        DiagnosticCode::IkInvariantFailed
    );
    assert_eq!(
        last.diagnostic.as_ref().unwrap().rule.as_deref(),
        Some("no_playing_muted")
    );
    for (code, rule) in [
        (
            DiagnosticCode::IkInvariantFailed,
            Some("invariants.some_other_rule.assert"),
        ),
        (DiagnosticCode::IkInvariantFailed, None),
        (DiagnosticCode::IkIllegalState, Some("no_playing_muted")),
        (
            DiagnosticCode::IkIllegalState,
            Some("invariants.some_other_rule.assert"),
        ),
    ] {
        let mut tampered = witness.clone();
        tampered.diagnostic.code = code;
        tampered.diagnostic.rule = rule.map(str::to_string);
        assert!(
            regions
                .project_invariant_witness(&machine, &tampered, &corpus)
                .is_err(),
            "changing only code/rule on a successfully replayed witness must refuse"
        );
    }
}

#[test]
fn terminal_siblings_are_preserved_but_cannot_be_joint_targets_or_explicit_sources() {
    let text = r#"schema_version = 2
machine_id = "terminal_sibling"
machine_version = 1
[regions.left.states]
open = { initial = true }
closed = { terminal = true }
[regions.right.states]
idle = { initial = true }
busy = {}
[regions.left.commands.close]
from = ["open"]
to = "closed"
[commands.work]
from = { right = ["idle"] }
to = { right = "busy" }
"#;
    let (machine, regions) = model(text);
    let closed = step(&machine, &machine.initial_state(), json!({"name":"close"}));
    let worked = step(&machine, &closed, json!({"name":"work"}));
    decoded(&regions, &worked, &[("left", "closed"), ("right", "busy")]);
    let explicit_terminal = text.replace(
        "from = { right = [\"idle\"] }",
        "from = { left = [\"closed\"], right = [\"idle\"] }",
    );
    refused(&explicit_terminal, "explicit terminal joint source");
    let targets_terminal_component = text.replace(
        "to = { right = \"busy\" }",
        "to = { left = \"open\", right = \"busy\" }",
    );
    let (targeting, _) = model(&targets_terminal_component);
    let terminal_sibling = step(
        &targeting,
        &targeting.initial_state(),
        json!({"name":"close"}),
    );
    assert!(targeting
        .decide(
            &terminal_sibling,
            None,
            &EffectStatus::None,
            &json!({"name":"work"})
        )
        .is_err());
}

#[test]
fn child_guard_evaluation_error_refuses_instead_of_falling_back_to_parent() {
    let text = format!(
        "{}\n[facts.flag]\ntype = 'bool'\npolicy = 'replace'\n",
        COMPOUND.replace("$override_enabled == true", "flag == true")
    );
    let (machine, _) = model(&text);
    let initial = machine.initial_state();
    let result = machine.decide(
        &initial,
        None,
        &EffectStatus::None,
        &json!({"name":"power_off", "args":{"override_enabled":false}}),
    );
    assert!(result.is_err(), "an unavailable child-guard fact is an error, not a false guard that enables parent fallback");
}

fn product_source3(a: usize, b: usize, c: usize) -> String {
    let mut text = "schema_version = 2\nmachine_id = 'products'\nmachine_version = 1\n".to_string();
    for (region, size) in [("a", a), ("b", b), ("c", c)] {
        text.push_str(&format!("[regions.{region}.states]\n"));
        for index in 0..size {
            text.push_str(&format!("s{index} = {{ initial = {} }}\n", index == 0));
        }
    }
    text
}

proptest::proptest! {
    #[test]
    fn region_pruned_products_match_generated_local_guard_graph(
        a_edges in 0u8..16u8,
        b_edges in 0u8..16u8,
        b_guard_bits in 0u8..=u8::MAX
    ) {
        let mut authored = r#"schema_version = 2
machine_id = "generated_products"
machine_version = 1
[regions.a.states]
a0 = { initial = true }
a1 = {}
[regions.b.states]
b0 = { initial = true }
b1 = {}
"#.to_string();
        for edge in 0..4 {
            let from = edge / 2;
            let to = edge % 2;
            if a_edges & (1u8 << edge) != 0 {
                authored.push_str(&format!(
                    "\n[regions.a.commands.edge_a_{from}_{to}]\nfrom = [\"a{from}\"]\nto = \"a{to}\"\n"
                ));
            }
            if b_edges & (1u8 << edge) != 0 {
                let selector = (b_guard_bits >> (2 * edge)) & 0b11;
                let guard = match selector {
                    0 => String::new(),
                    1 => "when = 'region_a == \"a0\"'\n".to_string(),
                    _ => "when = 'region_a == \"a1\"'\n".to_string(),
                };
                authored.push_str(&format!(
                    "\n[regions.b.commands.edge_b_{from}_{to}]\nfrom = [\"b{from}\"]\nto = \"b{to}\"\n{guard}"
                ));
            }
        }

        let source = parse_source(&authored).unwrap();
        let mut reached = BTreeSet::from([(0usize, 0usize)]);
        let mut frontier = vec![(0usize, 0usize)];
        while let Some((a, b)) = frontier.pop() {
            for edge in 0..4 {
                let from = edge / 2;
                let to = edge % 2;
                if a == from && a_edges & (1u8 << edge) != 0 && reached.insert((to, b)) {
                    frontier.push((to, b));
                }
                if b == from && b_edges & (1u8 << edge) != 0 {
                    let selector = (b_guard_bits >> (2 * edge)) & 0b11;
                    let guard_holds = selector == 0 || (selector == 1 && a == 0) || (selector >= 2 && a == 1);
                    if guard_holds && reached.insert((a, to)) {
                        frontier.push((a, to));
                    }
                }
            }
        }
        let expected = reached
            .iter()
            .map(|(a, b)| format!("a{a}__b{b}"))
            .collect::<BTreeSet<_>>();
        let actual = source.definition.states.keys().cloned().collect::<BTreeSet<_>>();
        proptest::prop_assert_eq!(actual, expected.clone());
        for command in source.definition.commands.values() {
            for from in &command.from {
                proptest::prop_assert!(expected.contains(from));
            }
            for outcome in command.outcomes.iter().flatten() {
                proptest::prop_assert!(source.definition.states.contains_key(&outcome.to));
            }
        }
    }
}

#[test]
fn region_product_limits_are_configurable_and_checked_before_expansion() {
    let default = parse_source(&product_source3(4, 4, 4)).unwrap();
    assert_eq!(default.definition.states.len(), 1);
    assert!(default.definition.states.contains_key("s0__s0__s0"));
    let too_large = product_source3(5, 13, 1);
    let refusal = parse_source(&too_large).unwrap_err();
    assert!(refusal[0].contains("a=5, b=13, c=1"));
    assert!(refusal[0].contains("raise region_product_bound"));
    assert!(refusal[0].contains("product exceeds 64 states"));
    let accepted = parse_source_with_limits(
        &too_large,
        RegionLimits {
            region_product_bound: 72,
        },
    )
    .unwrap();
    assert_eq!(accepted.definition.states.len(), 1);
    let mut huge = "schema_version = 2\nmachine_id = 'overflow'\nmachine_version = 1\n".to_string();
    for index in 0..128 {
        huge.push_str(&format!(
            "[regions.r{index:03}.states]\ns0 = {{ initial = true }}\ns1 = {{}}\n"
        ));
    }
    let overflow = parse_source_with_limits(
        &huge,
        RegionLimits {
            region_product_bound: usize::MAX,
        },
    )
    .unwrap_err();
    assert!(overflow[0].contains("product size overflow"));
    assert!(overflow[0].contains(&format!("configured bound {}", usize::MAX)));
}

#[test]
fn region_guards_keep_unknowns_and_drop_only_proven_same_region_contradictions() {
    let unknown = r#"schema_version = 2
machine_id = "unknown"
machine_version = 1
[facts.flag]
type = "bool"
policy = "replace"
initial = false
[regions.a.states]
a1 = { initial = true }
a2 = {}
[regions.a.commands.go]
from = ["a1"]
args = { enabled = "bool" }
outcomes = [
  { when = 'flag == true', to = "a2" },
  { when = '$enabled == true', to = "a2" },
]
"#;
    let parsed = parse_source(unknown).unwrap();
    let outcomes = parsed.definition.commands["go"].outcomes.as_ref().unwrap();
    assert_eq!(
        outcomes.len(),
        2,
        "domain facts and args remain unknown to lowering"
    );

    let known = BTreeMap::from([("region_a".to_string(), json!("a1"))]);
    let eval_guard = |guard| {
        let expression = ik::declarative::expr::parse(guard).unwrap();
        ik::declarative::expr::eval_with_assignments(&expression, &known)
    };
    assert_eq!(
        eval_guard("region_a == \"a2\" && $enabled == true"),
        Some(false)
    );
    assert_eq!(
        eval_guard("region_a == \"a1\" || $enabled == true"),
        Some(true)
    );
    assert_eq!(eval_guard("region_a == \"a2\" || $enabled == true"), None);
    assert_eq!(eval_guard("exists(region_a)"), Some(true));
    assert_eq!(eval_guard("not_exists(region_a)"), Some(false));
    assert_eq!(eval_guard("exists(flag)"), None);
    assert_eq!(eval_guard("not_exists(flag)"), None);
    assert_eq!(eval_guard("region_a + 1 == 2"), None);
    let contradictory = r#"schema_version = 2
machine_id = "contradiction"
machine_version = 1
[regions.a.states]
a1 = { initial = true }
a2 = {}
a3 = {}
[regions.a.commands.go]
from = ["a1"]
outcomes = [
  { when = 'region_a == "a2" && region_a == "a3"', to = "a2" },
]
"#;
    let parsed = parse_source(contradictory).unwrap();
    assert!(!parsed.definition.commands.contains_key("go"));
    let regions = parsed.regions.unwrap();
    assert!(regions.diagnostics.iter().any(|message| {
        message.contains("command `go`")
            && message.contains("product `a1`")
            && message.contains("region_a == \"a2\" && region_a == \"a3\"")
    }));
    assert_eq!(
        parsed.definition.states.keys().cloned().collect::<Vec<_>>(),
        vec!["a1"]
    );
    assert!(!parsed
        .definition
        .invariants
        .contains_key("region_projection_a2"));
}

proptest::proptest! {
    #[test]
    fn region_guard_connectives_match_generated_completion_consensus(
        mut kinds in proptest::collection::vec(0u8..4u8, 3..=5),
        first_is_or in proptest::bool::ANY
    ) {
        kinds[0] = 2;
        let mut unknown_index = 0usize;
        let unknowns = kinds
            .iter()
            .map(|kind| {
                if *kind >= 2 {
                    let index = unknown_index;
                    unknown_index += 1;
                    Some(index)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        let leaves = kinds
            .iter()
            .enumerate()
            .map(|(index, kind)| match *kind {
                0 | 1 => Expr::Cmp(Cmp {
                    left: Operand::Fact("region_a".into()),
                    op: CmpOp::Eq,
                    right: Operand::Literal(json!(if *kind == 0 { "a1" } else { "a2" })),
                }),
                2 => Expr::Exists(format!("flag_{index}")),
                3 => Expr::Cmp(Cmp {
                    left: Operand::Arg(format!("enabled_{index}")),
                    op: CmpOp::Eq,
                    right: Operand::Literal(json!(true)),
                }),
                _ => unreachable!(),
            })
            .collect::<Vec<_>>();
        let known = BTreeMap::from([("region_a".to_string(), json!("a1"))]);
        let connectives = (0..leaves.len() - 1)
            .map(|index| first_is_or ^ (index % 2 == 1))
            .collect::<Vec<_>>();
        let mut nested_expr = leaves[0].clone();
        for (leaf, is_or) in leaves.iter().skip(1).zip(&connectives) {
            nested_expr = if *is_or {
                Expr::Or(vec![nested_expr, leaf.clone()])
            } else {
                Expr::And(vec![nested_expr, leaf.clone()])
            };
        }
        let mut conjunction_completions = Vec::new();
        let mut disjunction_completions = Vec::new();
        let mut nested_completions = Vec::new();
        for assignment in 0..(1usize << unknown_index) {
            let values = kinds
                .iter()
                .enumerate()
                .map(|(index, kind)| match *kind {
                    0 => true,
                    1 => false,
                    _ => assignment & (1usize << unknowns[index].unwrap()) != 0,
                })
                .collect::<Vec<_>>();
            conjunction_completions.push(values.iter().all(|value| *value));
            disjunction_completions.push(values.iter().any(|value| *value));
            let mut nested = values[0];
            for (value, is_or) in values.iter().skip(1).zip(&connectives) {
                nested = if *is_or {
                    nested || *value
                } else {
                    nested && *value
                };
            }
            nested_completions.push(nested);
        }
        let consensus = |values: &[bool]| {
            if values.iter().all(|value| *value) {
                Some(true)
            } else if values.iter().all(|value| !*value) {
                Some(false)
            } else {
                None
            }
        };
        proptest::prop_assert_eq!(
            expr::eval_with_assignments(&Expr::And(leaves.clone()), &known),
            consensus(&conjunction_completions)
        );
        proptest::prop_assert_eq!(
            expr::eval_with_assignments(&Expr::Or(leaves), &known),
            consensus(&disjunction_completions)
        );
        proptest::prop_assert_eq!(
            expr::eval_with_assignments(&nested_expr, &known),
            consensus(&nested_completions)
        );
    }
}

#[test]
fn cross_region_couplings_prune_unreachable_products_and_accept() {
    const CROSS: &str = r#"schema_version = 2
machine_id = "cross"
machine_version = 1
[regions.a.states]
a1 = { initial = true }
a2 = {}
[regions.a.commands.go_a]
from = ["a1"]
to = "a2"
[regions.b.states]
b1 = { initial = true }
b2 = {}
[regions.b.commands.go_b]
from = ["b1"]
to = "b2"
when = 'region_a == "a2"'
"#;
    const JOINT: &str = r#"schema_version = 2
machine_id = "joint"
machine_version = 1
[regions.a.states]
a1 = { initial = true }
a2 = {}
[regions.a.commands.go_a]
from = ["a1"]
to = "a2"
[regions.b.states]
b1 = { initial = true }
b2 = {}
[commands.coupled]
from = { b = ["b1"] }
when = 'region_a == "a2"'
to = { b = "b2" }
"#;
    for authored in [CROSS, JOINT] {
        let first = parse_source(authored).unwrap();
        let second = parse_source(authored).unwrap();
        assert_eq!(
            serde_json::to_value(&first.definition).unwrap(),
            serde_json::to_value(&second.definition).unwrap(),
            "product IDs and output are deterministic"
        );
        let states = first
            .definition
            .states
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>();
        assert_eq!(
            states,
            BTreeSet::from(["a1__b1".into(), "a2__b1".into(), "a2__b2".into()])
        );
        assert!(!first.definition.states.contains_key("a1__b2"));
        assert!(!first
            .definition
            .invariants
            .contains_key("region_projection_a1__b2"));
        for command in first.definition.commands.values() {
            assert!(!command.from.contains(&"a1__b2".to_string()));
            for outcome in command.outcomes.iter().flatten() {
                assert!(first.definition.states.contains_key(&outcome.to));
            }
        }
        let lowering_diagnostics = first.regions.as_ref().unwrap().diagnostics.join("\n");
        assert!(lowering_diagnostics.contains("a1__b2"));
        assert!(lowering_diagnostics.contains("a=a1, b=b2"));
        assert!(first
            .regions
            .as_ref()
            .unwrap()
            .decode(&json!({"state":"a1__b2", "region_a":"a1", "region_b":"b2"}))
            .is_err());
        let checked = ik::declarative::region::check_source(
            &first,
            None,
            &CaseCorpus::new(),
            &[],
            Bounds {
                max_nodes: 128,
                max_depth: 8,
            },
        )
        .unwrap();
        // The previous REFUSED pin was incidental: it demanded a successful
        // commit from a1__b1 after lowering had already removed that edge.
        // Feasible commands still execute; the impossible source is recorded.
        assert_eq!(checked.verdict, "ACCEPT", "{}", checked.text);
        let impossible_id = if authored.contains("go_b") {
            "command/go_b/from/a1__b1/executable"
        } else {
            "command/coupled/from/a1__b1/executable"
        };
        let recorded = checked
            .flat
            .report
            .impossibilities
            .iter()
            .find(|row| row.obligation_id == impossible_id)
            .unwrap_or_else(|| {
                panic!(
                    "missing impossibility for {impossible_id}: {}",
                    checked.text
                )
            });
        assert_eq!(recorded.class, "command_source");
        assert!(recorded.discharged);
        assert!(
            !checked
                .flat
                .report
                .obligations
                .uncovered_ids
                .iter()
                .any(|id| id == impossible_id),
            "impossible success demanded as uncovered: {}",
            checked.text
        );
        assert!(
            checked.text.contains("IMPOSSIBLE SUCCESS"),
            "{}",
            checked.text
        );
        assert!(checked.text.contains("REGION LOWERING DIAGNOSTICS"));
        assert!(checked.text.contains("corpus-independent"));
    }
}

#[test]
fn shop_local_and_joint_couplings_prune_products_and_name_the_cause() {
    const COMMON: &str = r#"schema_version = 2
machine_id = "shop"
machine_version = 1
[regions.order.states]
draft = { initial = true }
placed = {}
shipped = {}
cancelled = { terminal = true }
[regions.order.commands.place]
from = ["draft"]
to = "placed"
[regions.order.commands.ship]
from = ["placed"]
to = "shipped"
[regions.order.commands.cancel]
from = ["draft"]
to = "cancelled"
[regions.payment.states]
idle = { initial = true }
captured = {}
refunded = {}
[regions.payment.commands.refund]
from = ["captured"]
to = "refunded"
"#;
    let local = format!(
        "{COMMON}\n[regions.payment.commands.capture]\nfrom = [\"idle\"]\nto = \"captured\"\nwhen = 'region_order == \"placed\"'\n"
    );
    let joint = format!(
        "{COMMON}\n[commands.capture]\nfrom = {{ order = [\"placed\"], payment = [\"idle\"] }}\nto = {{ payment = \"captured\" }}\n"
    );
    let expected = BTreeSet::from([
        "cancelled__idle".to_string(),
        "draft__idle".to_string(),
        "placed__captured".to_string(),
        "placed__idle".to_string(),
        "placed__refunded".to_string(),
        "shipped__captured".to_string(),
        "shipped__idle".to_string(),
        "shipped__refunded".to_string(),
    ]);
    for (authored, cause, impossible_id) in [
        (
            local,
            "region guard `region_order == \"placed\"`",
            Some("command/capture/from/draft__idle/executable"),
        ),
        (joint, "requires order in [placed]", None),
    ] {
        let source = parse_source(&authored).unwrap();
        let actual = source
            .definition
            .states
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>();
        assert_eq!(actual, expected);
        for command in source.definition.commands.values() {
            for outcome in command.outcomes.iter().flatten() {
                assert!(source.definition.states.contains_key(&outcome.to));
            }
        }
        let diagnostics = source.regions.as_ref().unwrap().diagnostics.join("\n");
        assert!(diagnostics.contains("draft__captured"), "{diagnostics}");
        assert!(diagnostics.contains(cause), "{diagnostics}");
        let checked = ik::declarative::region::check_source(
            &source,
            None,
            &CaseCorpus::new(),
            &[],
            Bounds {
                max_nodes: 128,
                max_depth: 8,
            },
        )
        .unwrap();
        // Local capture from draft__idle is a legal refusal source whose success
        // edge was folded away. That used to refuse the check as uncovered
        // success debt; the contract records impossibility and accepts the
        // remaining feasible commands.
        assert_eq!(checked.verdict, "ACCEPT", "{}", checked.text);
        if let Some(impossible_id) = impossible_id {
            assert!(
                checked
                    .flat
                    .report
                    .impossibilities
                    .iter()
                    .any(|row| row.obligation_id == impossible_id),
                "missing impossibility for {impossible_id}: {}",
                checked.text
            );
        }
        assert!(checked.text.contains(cause), "{}", checked.text);
    }
}
