//! Chart is the authored source of truth; spec_hash stays the flat digest.

use ik::canonical::digest_json;
use ik::declarative::chart::{
    analyze_chart, check_consistent, drop_first_edge, Chart, ChartError,
};
use ik::declarative::{load_definition, parse_definition};
use std::path::PathBuf;

fn repo(path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(path)
}

const TAGGED_HASH: &str = "f0bef35d059f09af32614c758cbfb5f66ac707bf92c03240dd96b729cb3758d6";
const LOAN_HASH: &str = "6c08d4be51065bc9fd184c78009f7ef1f3b5c40f82e1d2d9d46e226cabaa15d8";

fn fixtures() -> Vec<PathBuf> {
    vec![
        repo("validation/dfy/journeys/fixtures/loan_checkout.ik.toml"),
        repo("validation/dfy/journeys/fixtures/loan_v2.ik.toml"),
        repo("examples/shipped/tagged_item.ik.toml"),
        repo("machines/region_parallel.ik.toml"),
        repo("machines/region_compound.ik.toml"),
        repo("tests/fixtures/diagram/region_stacked.ik.toml"),
        repo("tests/fixtures/diagram/coffee_compound.ik.toml"),
    ]
}

#[test]
fn tagged_item_and_loan_spec_hashes_are_stable() {
    let tagged = load_definition(&repo("examples/shipped/tagged_item.ik.toml")).expect("tagged");
    let loan = load_definition(&repo("validation/dfy/journeys/fixtures/loan_checkout.ik.toml"))
        .expect("loan");
    assert_eq!(digest_json(&tagged), TAGGED_HASH);
    assert_eq!(digest_json(&loan), LOAN_HASH);
}

#[test]
fn spec_hash_equals_flat_digest_for_region_fixtures() {
    for path in fixtures() {
        let def = load_definition(&path).expect("def");
        let before = digest_json(&def);
        let again = load_definition(&path).expect("again");
        assert_eq!(before, digest_json(&again), "{}", path.display());
        assert_eq!(before, digest_json(&def), "{}", path.display());
    }
}

#[test]
fn every_fixture_chart_is_consistent_with_flat() {
    for path in fixtures() {
        let def = load_definition(&path).expect("def");
        assert_eq!(
            check_consistent(&def.chart, &def),
            Ok(()),
            "{}",
            path.display()
        );
        let keys = def.chart.authored_keys();
        assert_eq!(keys.is_empty(), false, "{}", path.display());
    }
}

#[test]
fn dropping_an_authored_edge_is_refused() {
    let mut def =
        load_definition(&repo("tests/fixtures/diagram/coffee_compound.ik.toml")).expect("coffee");
    let dropped = drop_first_edge(&mut def.chart).expect("edge");
    let err = check_consistent(&def.chart, &def).expect_err("must refuse");
    assert_eq!(err.code, "IK_CHART_INCONSISTENT");
    assert_eq!(err.message.contains(&dropped.command), true, "{err}");
}

#[test]
fn coffee_chart_has_six_commands_and_no_ends() {
    let def =
        load_definition(&repo("tests/fixtures/diagram/coffee_compound.ik.toml")).expect("coffee");
    assert_eq!(def.chart.hierarchical, true);
    assert_eq!(def.chart.regions.len(), 1);
    let power = def.chart.regions.get("power").expect("power");
    assert_eq!(power.states.len(), 5);
    assert_eq!(power.groups.len(), 1);
    assert_eq!(power.groups.get("on").expect("on").parent_edges.len(), 1);
    let mut commands = std::collections::BTreeSet::new();
    for key in def.chart.authored_keys() {
        commands.insert(key.command);
    }
    assert_eq!(commands.len(), 6);
    let analytics = analyze_chart(&def.chart);
    assert_eq!(analytics.ends.is_empty(), true, "{:?}", analytics.ends);
}

#[test]
fn schema1_chart_is_one_region() {
    let def = parse_definition(
        r#"
schema_version = 1
machine_id = "pair"
machine_version = 1
[facts]
[states]
idle = { initial = true }
done = { terminal = true }
[commands.go]
from = ["idle"]
to = "done"
"#,
    )
    .expect("pair");
    assert_eq!(def.chart.hierarchical, false);
    assert_eq!(def.chart.regions.len(), 1);
    assert_eq!(check_consistent(&def.chart, &def), Ok(()));
}

#[test]
fn chart_error_names_the_code() {
    let err = ChartError {
        code: "IK_CHART_INCONSISTENT",
        message: "x".into(),
    };
    assert_eq!(format!("{err}"), "IK_CHART_INCONSISTENT: x");
}

#[test]
fn from_flat_matches_parse() {
    let def = load_definition(&repo("examples/shipped/tagged_item.ik.toml")).expect("tagged");
    let rebuilt = Chart::from_flat(&def);
    assert_eq!(rebuilt.authored_keys(), def.chart.authored_keys());
}
