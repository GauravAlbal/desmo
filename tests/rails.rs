//! Return rails: back-edges drawn in a left gutter, annotation stays, budget of two.

use ik::declarative::{load_definition, parse_definition, Definition};
use ik::diagram::{build, display_width, DiagramKind};
use std::path::PathBuf;

fn repo(path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(path)
}

/// s0 -> s1 -> s2 -> s3 -> done with the given back-edges (command, from, to).
fn line_machine(back: &[(&str, &str, &str)]) -> Definition {
    let mut toml = String::from(
        "schema_version = 1\nmachine_id = \"line\"\nmachine_version = 1\n[facts]\n[states]\n\
         s0 = { initial = true }\ns1 = {}\ns2 = {}\ns3 = {}\ndone = { terminal = true }\n",
    );
    for (name, from, to) in [
        ("go0", "s0", "s1"),
        ("go1", "s1", "s2"),
        ("go2", "s2", "s3"),
        ("finish", "s3", "done"),
    ] {
        toml.push_str(&format!(
            "[commands.{name}]\nfrom = [\"{from}\"]\nto = \"{to}\"\n"
        ));
    }
    for (name, from, to) in back {
        toml.push_str(&format!(
            "[commands.{name}]\nfrom = [\"{from}\"]\nto = \"{to}\"\n"
        ));
    }
    parse_definition(&toml).expect("line machine")
}

fn map_of(def: &Definition) -> String {
    build(Some(def), None, &[], DiagramKind::Delivery, &def.machine_id, None).to_route_map()
}

fn corners(map: &str, glyph: char) -> usize {
    map.chars().filter(|c| *c == glyph).count()
}

fn row_of<'a>(map: &'a str, needle: &str) -> &'a str {
    map.lines().find(|line| line.contains(needle)).unwrap_or("")
}

#[test]
fn single_back_edge_draws_one_rail_into_its_target() {
    let map = map_of(&line_machine(&[("retry", "s2", "s0")]));
    assert_eq!(corners(&map, '┌'), 1, "{map}");
    assert_eq!(corners(&map, '└'), 1, "{map}");
    assert_eq!(row_of(&map, "[ s0 ]").contains("┌──▶● [ s0 ]"), true, "{map}");
    assert_eq!(row_of(&map, "retry").starts_with(" └───────"), true, "{map}");
    assert_eq!(map.matches("retry").count(), 1, "{map}");
}

#[test]
fn rail_rows_between_corners_carry_the_vertical() {
    let map = map_of(&line_machine(&[("retry", "s2", "s0")]));
    let lines: Vec<&str> = map.lines().collect();
    let top = lines.iter().position(|l| l.contains("┌──▶")).expect("top");
    let bottom = lines.iter().position(|l| l.contains("└────")).expect("bottom");
    let between = &lines[top + 1..bottom];
    assert_eq!(between.is_empty(), false, "{map}");
    assert_eq!(between.iter().all(|l| l.starts_with(" │")), true, "{map}");
}

#[test]
fn nested_back_edges_use_two_columns_inner_nearest_the_river() {
    let map = map_of(&line_machine(&[("outer", "s3", "s0"), ("inner", "s2", "s1")]));
    assert_eq!(corners(&map, '┌'), 2, "{map}");
    assert_eq!(corners(&map, '└'), 2, "{map}");
    let outer = row_of(&map, "[ s0 ]");
    let inner = row_of(&map, "[ s1 ]");
    assert_eq!(outer.starts_with(" ┌────▶●"), true, "{outer}");
    assert_eq!(inner.starts_with(" │ ┌──▶○"), true, "{inner}");
}

#[test]
fn crossing_back_edges_keep_the_second_as_an_annotation() {
    let map = map_of(&line_machine(&[("first", "s2", "s0"), ("second", "s3", "s1")]));
    assert_eq!(corners(&map, '┌'), 1, "{map}");
    assert_eq!(map.matches("first").count(), 1, "{map}");
    assert_eq!(map.matches("second").count(), 1, "{map}");
    assert_eq!(row_of(&map, "second").trim_start().starts_with('↗'), true, "{map}");
}

#[test]
fn a_third_rail_is_not_drawn_but_stays_labelled() {
    let map = map_of(&line_machine(&[
        ("r0", "s1", "s0"),
        ("r1", "s2", "s1"),
        ("r2", "s3", "s2"),
    ]));
    assert_eq!(corners(&map, '┌') <= 2, true, "{map}");
    for name in ["r0", "r1", "r2"] {
        assert_eq!(map.matches(name).count(), 1, "{map}");
    }
}

#[test]
fn machines_without_back_edges_get_no_gutter() {
    let map = map_of(&line_machine(&[]));
    assert_eq!(corners(&map, '┌'), 0, "{map}");
    assert_eq!(map.lines().any(|line| line.starts_with("  ● [ s0 ]")), true, "{map}");
}

#[test]
fn coffee_take_cup_rail_stays_inside_the_group_box() {
    let def = load_definition(&repo("tests/fixtures/diagram/coffee_compound.ik.toml")).expect("coffee");
    let map = map_of(&def);
    assert_eq!(map.contains("┌──▶● [ idle ]"), true, "{map}");
    assert_eq!(map.contains("└────↺ take_cup → [ idle ]"), true, "{map}");
    let widths: Vec<usize> = map
        .lines()
        .filter(|line| line.starts_with('│') || line.starts_with('┌') || line.starts_with('└'))
        .map(display_width)
        .collect();
    assert_eq!(widths.windows(2).all(|pair| pair[0] == pair[1]), true, "{map}");
}

#[test]
fn rails_render_identically_when_commands_are_declared_in_another_order() {
    let a = map_of(&line_machine(&[("outer", "s3", "s0"), ("inner", "s2", "s1")]));
    let b = map_of(&line_machine(&[("inner", "s2", "s1"), ("outer", "s3", "s0")]));
    assert_eq!(a, b);
}

#[test]
fn coffee_route_map_is_golden() {
    let def = load_definition(&repo("tests/fixtures/diagram/coffee_compound.ik.toml")).expect("coffee");
    let golden = std::fs::read_to_string(repo("tests/fixtures/diagram/coffee_compound.route.txt"))
        .expect("golden");
    assert_eq!(map_of(&def).trim_end(), golden.trim_end());
}
