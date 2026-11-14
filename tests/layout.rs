//! In-house river layout: stable under insertion shuffle, width, labels.

use ik::declarative::load_definition;
use ik::diagram::{
    build, command_counts, display_width, layout_chart, layout_graph, DrawnEdge, DiagramKind,
};
use std::path::PathBuf;

fn repo(path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(path)
}

fn shuffle<T>(items: &mut [T], mut seed: u64) {
    for i in (1..items.len()).rev() {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let j = (seed as usize) % (i + 1);
        items.swap(i, j);
    }
}

const SEEDS: [u64; 8] = [1, 2, 7, 13, 42, 99, 255, 1024];

fn sample_graph() -> (Vec<String>, Vec<DrawnEdge>) {
    let nodes = vec![
        "idle".into(),
        "grinding".into(),
        "brewing".into(),
        "ready".into(),
        "off".into(),
    ];
    let edges = vec![
        edge("start", "idle", "grinding"),
        edge("grind_complete", "grinding", "brewing"),
        edge("brew_complete", "brewing", "ready"),
        edge("take_cup", "ready", "idle"),
        edge("power_on", "off", "idle"),
        edge("power_off", "idle", "off"),
        edge("power_off", "grinding", "off"),
    ];
    (nodes, edges)
}

fn edge(command: &str, from: &str, to: &str) -> DrawnEdge {
    DrawnEdge {
        command: command.into(),
        from: from.into(),
        to: to.into(),
    }
}

#[test]
fn layout_identical_under_insertion_shuffle() {
    let (nodes, edges) = sample_graph();
    let base = layout_graph(nodes.clone(), edges.clone(), Some("idle"));
    for seed in SEEDS {
        let mut n = nodes.clone();
        let mut e = edges.clone();
        shuffle(&mut n, seed);
        shuffle(&mut e, seed.wrapping_mul(17));
        let other = layout_graph(n, e, Some("idle"));
        assert_eq!(other.spine, base.spine, "seed {seed}");
        assert_eq!(other.drawn, base.drawn, "seed {seed}");
        assert_eq!(other.paired, base.paired, "seed {seed}");
    }
}

#[test]
fn drawn_list_matches_chart_authored_edges() {
    let paths = [
        "machines/region_parallel.ik.toml",
        "machines/region_compound.ik.toml",
        "tests/fixtures/diagram/region_stacked.ik.toml",
        "tests/fixtures/diagram/coffee_compound.ik.toml",
        "examples/shipped/tagged_item.ik.toml",
        "validation/dfy/journeys/fixtures/loan_checkout.ik.toml",
    ];
    for path in paths {
        let def = load_definition(&repo(path)).expect(path);
        let mut drawn = layout_chart(&def.chart);
        drawn.sort();
        let mut authored: Vec<DrawnEdge> = def
            .chart
            .authored_keys()
            .into_iter()
            .map(|key| DrawnEdge {
                command: key.command,
                from: key.from,
                to: key.to,
            })
            .collect();
        authored.sort();
        authored.dedup();
        assert_eq!(drawn, authored, "{path}");
        let diagram = build(
            Some(&def),
            None,
            &[],
            DiagramKind::Delivery,
            &def.machine_id,
            None,
        );
        let map = diagram.to_route_map();
        let counts = command_counts(&drawn);
        for (command, expect) in counts {
            let got = map.match_indices(command.as_str()).count();
            assert_eq!(
                got >= 1,
                true,
                "{path} dropped {command} (want {expect}, map has {got})\n{map}"
            );
        }
        for line in map.lines() {
            assert_eq!(
                display_width(line) <= 120,
                true,
                "{path} width {}\n{line}",
                display_width(line)
            );
        }
        let json = diagram.json_field();
        let listed = json["drawn_edges"].as_array().expect("drawn_edges");
        assert_eq!(listed.is_empty(), false, "{path}");
        for item in listed {
            let cmd = item["command"].as_str().unwrap_or("");
            assert_eq!(map.contains(cmd), true, "{path} json {cmd}");
        }
        if def.chart.hierarchical {
            assert_eq!(diagram.drawn_edge_list(), authored);
        }
    }
}

#[test]
fn two_cycle_pairs_when_adjacent() {
    let nodes = vec!["paused".into(), "playing".into()];
    let edges = vec![
        edge("pause", "playing", "paused"),
        edge("play", "paused", "playing"),
    ];
    let layout = layout_graph(nodes, edges, Some("paused"));
    assert_eq!(layout.paired.len(), 1);
    assert_eq!(layout.spine.len(), 2);
}
