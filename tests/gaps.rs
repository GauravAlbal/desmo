//! Gaps derived from a definition alone. Each kind must fire on a definition built to have
//! it, and must stay quiet on a definition that does not.

use ik::declarative::gaps::{definition_gaps, Gap};
use ik::declarative::load_definition;
use std::fs;
use std::path::PathBuf;

fn load(name: &str, text: &str) -> ik::declarative::Definition {
    let dir = std::env::temp_dir().join(format!("ik-gaps-{name}-{}", std::process::id()));
    fs::create_dir_all(&dir).expect("dir");
    let path: PathBuf = dir.join("machine.ik.toml");
    fs::write(&path, text).expect("write");
    load_definition(&path).unwrap_or_else(|errors| panic!("{}", errors.join("; ")))
}

fn kinds(gaps: &[Gap]) -> Vec<(&'static str, String)> {
    gaps.iter().map(|gap| (gap.kind, gap.subject.clone())).collect()
}

const GAPPY: &str = r#"
schema_version = 1
machine_id = "gappy"
machine_version = 1

[facts]
orphan = { type = "int", policy = "replace" }
ghost = { type = "int", policy = "replace" }
kept = { type = "int", policy = "replace", initial = 0 }

[states]
a = { initial = true }
b = { terminal = true }
dead = {}
lost = {}

[commands.go]
from = ["a"]
to = "b"
args = { x = "int" }
then = { orphan = "1" }

[commands.stray]
from = ["a"]
to = "dead"
when = "kept == 0"

[commands.reclaim]
from = ["lost"]
to = "b"
when = "ghost == 1"

[commands.plan]
from = ["a"]
to = "b"
args = { eid = "string" }
when = "$eid == $eid"
"#;

#[test]
fn each_kind_fires_on_a_definition_built_to_have_it() {
    let gaps = definition_gaps(&load("gappy", GAPPY));
    let found = kinds(&gaps);
    for expected in [
        ("unguarded_argument", "go $x"),
        ("unguarded_command", "go"),
        ("fact_never_read", "orphan"),
        ("fact_never_set", "ghost"),
        ("dead_end", "dead"),
        ("unreachable_state", "lost"),
        ("no_invariants", "machine"),
    ] {
        assert_eq!(found.contains(&(expected.0, expected.1.to_string())), true, "missing {expected:?} in {found:?}");
    }
}

#[test]
fn a_fact_a_guard_reads_and_a_guarded_argument_are_not_gaps() {
    let found = kinds(&definition_gaps(&load("gappy2", GAPPY)));
    assert_eq!(found.contains(&("fact_never_read", "kept".to_string())), false, "{found:?}");
    assert_eq!(found.contains(&("unguarded_argument", "plan $eid".to_string())), false, "{found:?}");
    assert_eq!(found.contains(&("unguarded_command", "stray".to_string())), false, "{found:?}");
}

#[test]
fn material_gaps_come_first() {
    let gaps = definition_gaps(&load("gappy3", GAPPY));
    let first_other = gaps.iter().position(|gap| !gap.material).unwrap_or(gaps.len());
    assert_eq!(gaps[first_other..].iter().any(|gap| gap.material), false, "{:?}", kinds(&gaps));
}

#[test]
fn a_definition_with_an_invariant_and_no_loose_ends_has_no_gaps() {
    let tight = r#"
schema_version = 1
machine_id = "tight"
machine_version = 1

[facts]
n = { type = "int", policy = "replace", initial = 0 }

[states]
a = { initial = true }
b = { terminal = true }

[commands.go]
from = ["a"]
to = "b"
args = { v = "int" }
when = "$v > n"
then = { n = "$v" }

[invariants.nonneg]
assert = "n >= 0"
"#;
    assert_eq!(kinds(&definition_gaps(&load("tight", tight))), Vec::<(&'static str, String)>::new());
}
