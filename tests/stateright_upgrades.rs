//! Integration tests for Stateright-inspired exploration, simulation, property,
//! boundary, and simulator explorer features.

use ik::declarative::explore::{explore, explore_with_options, Bounds, CaseCorpus, ExploreOptions};
use ik::declarative::sim_check::{run_simulation, SimOptions};
use std::fs;
use std::process::Command;
use tempfile::TempDir;

fn ik(args: &[&str]) -> (String, String, i32) {
    let out = Command::new(env!("CARGO_BIN_EXE_ik"))
        .args(args)
        .output()
        .expect("run ik");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}

#[test]
fn inline_reach_preserves_schema1_spec_hash_and_validates_statically() {
    let orig_src = fs::read_to_string("machines/verification_attempt.ik.toml").expect("read base");
    let orig = ik::declarative::parse_and_compile(&orig_src).expect("compile base");

    let with_reach_src = format!(
        "{orig_src}\n[reach]\ncan_verify = \"state == 'verified'\"\n"
    );
    let with_reach = ik::declarative::parse_and_compile(&with_reach_src).expect("compile with reach");
    assert_eq!(orig.spec_hash(), with_reach.spec_hash());

    let bad_reach = r#"
schema_version = 1
machine_id = "bad_reach"
machine_version = 1

[states]
start = { initial = true }
done = { terminal = true }

[commands.noop]
from = ["start"]
to = "done"

[reach]
bad_state = "state == 'nonexistent'"
bad_arg = "$x == 1"
"#;
    let errs = ik::declarative::parse_and_compile(bad_reach).err().expect("must fail static check");
    let joined = errs.join(" | ");
    assert!(joined.contains("nonexistent"), "{joined}");
    assert!(joined.contains("cannot reference command argument"), "{joined}");
}

#[test]
fn inline_reach_satisfies_and_refuses_in_cli_and_schema2() {
    let dir = TempDir::new().expect("tempdir");

    let ok_path = dir.path().join("reach_ok.ik.toml");
    fs::write(
        &ok_path,
        r#"
schema_version = 1
machine_id = "reach_ok"
machine_version = 1

[facts]
paid_ref = { type = "string", policy = "replace" }

[states]
open = { initial = true }
done = { terminal = true }

[commands.finish]
from = ["open"]
to = "done"
args = { paid_ref = "string" }
then = { paid_ref = "$paid_ref" }

[reach]
can_finish = "state == 'done' && paid_ref != ''"
"#,
    )
    .unwrap();
    let (stdout, stderr, code) = ik(&[
        "check",
        ok_path.to_str().unwrap(),
        "--auto-seed",
        "--summary",
    ]);
    assert_eq!(code, 0, "stdout={stdout}\nstderr={stderr}");
    assert!(stdout.contains("[ok] can_finish"), "{stdout}");

    let miss_path = dir.path().join("reach_miss.ik.toml");
    fs::write(
        &miss_path,
        r#"
schema_version = 1
machine_id = "reach_miss"
machine_version = 1

[facts]
flag = { type = "bool", policy = "replace", initial = false }

[states]
open = { initial = true }
done = { terminal = true }

[commands.finish]
from = ["open"]
to = "done"

[reach]
never_set = "state == 'done' && flag == true"
"#,
    )
    .unwrap();
    let (stdout, stderr, code) = ik(&[
        "check",
        miss_path.to_str().unwrap(),
        "--auto-seed",
        "--summary",
    ]);
    assert_eq!(code, 1, "stdout={stdout}\nstderr={stderr}");
    assert!(stdout.contains("[MISS] never_set"), "{stdout}");

    let reg_path = dir.path().join("reach_reg.ik.toml");
    fs::write(
        &reg_path,
        r#"
schema_version = 2
machine_id = "reach_reg"
machine_version = 1

[regions.a.states]
a0 = { initial = true }
a1 = { terminal = true }

[regions.a.commands.step_a]
from = ["a0"]
to = "a1"

[regions.b.states]
b0 = { initial = true }
b1 = { terminal = true }

[regions.b.commands.step_b]
from = ["b0"]
to = "b1"

[reach]
both_done = "state == 'a1__b1'"
"#,
    )
    .unwrap();
    let (stdout, stderr, code) = ik(&[
        "check",
        reg_path.to_str().unwrap(),
        "--auto-seed",
        "--json",
    ]);
    assert_eq!(code, 0, "stdout={stdout}\nstderr={stderr}");
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("json");
    assert_eq!(json["reach_results"][0]["name"], "both_done");
    assert_eq!(json["reach_results"][0]["reached"], true);
}

#[test]
fn explicit_bound_completes_counter_bfs_and_prunes_outside_states() {
    let dir = TempDir::new().expect("tempdir");
    let path = dir.path().join("counter.ik.toml");
    fs::write(
        &path,
        r#"
schema_version = 1
machine_id = "bounded_counter"
machine_version = 1

[facts]
n = { type = "int", policy = "counter", initial = 0 }

[states]
looping = { initial = true }
done = { terminal = true }

[commands.inc]
from = ["looping"]
to = "looping"
then = { n = "+1" }

[commands.finish]
from = ["looping"]
to = "done"
"#,
    )
    .unwrap();

    // Without --bound under depth 16, counter truncation at max_depth makes search incomplete.
    let (out_unbounded, _, code_unbounded) =
        ik(&["check", path.to_str().unwrap(), "--auto-seed", "--json"]);
    assert_eq!(code_unbounded, 0);
    let j1: serde_json::Value = serde_json::from_str(&out_unbounded).unwrap();
    assert_eq!(j1["release"]["provenance"]["exploration_complete"], false);

    // With --bound "n <= 3", exploration completes exhaustively inside n <= 3 and prunes n == 4.
    let (out_bounded, err_bounded, code_bounded) = ik(&[
        "check",
        path.to_str().unwrap(),
        "--auto-seed",
        "--bound",
        "n <= 3",
        "--json",
    ]);
    assert_eq!(code_bounded, 0, "err={err_bounded}");
    let j2: serde_json::Value = serde_json::from_str(&out_bounded).unwrap();
    assert_eq!(j2["release"]["provenance"]["exploration_complete"], true);
    assert!(j2["boundary_pruned"].as_u64().unwrap_or(0) >= 1, "{j2}");

    // Bad --bound expression fails loudly with exit 2.
    let (_, bad_err, bad_code) = ik(&[
        "check",
        path.to_str().unwrap(),
        "--bound",
        "missing_fact <= 3",
    ]);
    assert_eq!(bad_code, 2);
    assert!(bad_err.contains("missing_fact"), "{bad_err}");
}

#[test]
fn seeded_simulation_finds_deep_bug_beyond_bfs_depth_and_emits_runnable_scenario() {
    let dir = TempDir::new().expect("tempdir");
    let path = dir.path().join("deep_bug.ik.toml");
    // Require 20 increments before `finish` violates invariant, beyond default BFS max_depth=16.
    fs::write(
        &path,
        r#"
schema_version = 1
machine_id = "deep_bug"
machine_version = 1

[facts]
steps = { type = "int", policy = "counter", initial = 0 }

[states]
running = { initial = true }
done = { terminal = true }

[invariants.no_deep_finish]
assert = "state != 'done' || steps < 20"

[commands.tick]
from = ["running"]
to = "running"
then = { steps = "+1" }

[commands.finish]
from = ["running"]
to = "done"
when = "steps >= 20"
"#,
    )
    .unwrap();

    // Verify default BFS bounds (4096, 16) do not hit the depth-21 invariant violation.
    let compiled = ik::declarative::load_def(&path).unwrap();
    let (bfs_run, bfs_refusals) = ik::declarative::explore::explore_with_invariant_witnesses(
        &compiled,
        &CaseCorpus::new(),
        Bounds {
            max_nodes: 4096,
            max_depth: 16,
        },
    );
    assert!(!bfs_run.complete);
    assert!(bfs_refusals.is_empty());

    // Direct SimulationChecker catches the deep violation deterministically with --seed + --steps.
    let sim_opts = SimOptions {
        seed: 42,
        max_steps: 40,
        traces: 256,
        fail_fast: true,
        boundary: Vec::new(),
    };
    let sim_a = run_simulation(&compiled, &CaseCorpus::new(), &sim_opts);
    let sim_b = run_simulation(&compiled, &CaseCorpus::new(), &sim_opts);
    assert_eq!(sim_a, sim_b);
    assert!(!sim_a.invariant_refusals.is_empty());

    // CLI auto-fallback (and explicit --simulate --emit-scenario) finds the deep violation and emits runnable scenario.json.
    let scenario_path = dir.path().join("repro.scenario.json");
    let (stdout, stderr, code) = ik(&[
        "check",
        path.to_str().unwrap(),
        "--simulate",
        "--seed",
        "42",
        "--steps",
        "40",
        "--traces",
        "256",
        "--summary",
        "--emit-scenario",
        scenario_path.to_str().unwrap(),
    ]);
    assert_eq!(code, 1, "stdout={stdout}\nstderr={stderr}");
    assert!(stdout.contains("invariant violations witnessed"), "{stdout}");
    assert!(stdout.contains("REFUSED (IK_INVARIANT_FAILED"), "{stdout}");
    assert!(scenario_path.is_file());

    // Emitted scenario executes cleanly under `ik scenario run`.
    let out_dir = dir.path().join("scenario-bundle");
    let (scen_out, scen_err, scen_code) = ik(&[
        "scenario",
        "run",
        path.to_str().unwrap(),
        scenario_path.to_str().unwrap(),
        "--out",
        out_dir.to_str().unwrap(),
    ]);
    assert_eq!(scen_code, 0, "stdout={scen_out}\nstderr={scen_err}");
    assert!(scen_out.contains("all expectations met"), "{scen_out}");
}

#[test]
fn parallel_threads_match_single_thread_and_fail_fast_aborts_early() {
    let src = r#"
schema_version = 1
machine_id = "parallel_parity"
machine_version = 1

[facts]
count = { type = "int", policy = "counter", initial = 0 }

[states]
start = { initial = true }
mid = {}
done = { terminal = true }

[invariants.cap]
assert = "state != 'done' || count <= 1"

[commands.step]
from = ["start", "mid"]
to = "mid"
then = { count = "+1" }

[commands.finish]
from = ["mid"]
to = "done"
"#;
    let machine = ik::declarative::parse_and_compile(src).unwrap();
    let bounds = Bounds {
        max_nodes: 256,
        max_depth: 6,
    };
    let (seq_run, seq_refusals) = explore_with_options(
        &machine,
        &CaseCorpus::new(),
        bounds,
        &ExploreOptions {
            threads: 1,
            ..Default::default()
        },
        true,
    );
    let (par_run, par_refusals) = explore_with_options(
        &machine,
        &CaseCorpus::new(),
        bounds,
        &ExploreOptions {
            threads: 4,
            ..Default::default()
        },
        true,
    );
    assert_eq!(seq_run, par_run);
    assert_eq!(seq_refusals, par_refusals);
    assert_eq!(seq_run, explore(&machine, &CaseCorpus::new(), bounds));

    let (ff_run, ff_refusals) = explore_with_options(
        &machine,
        &CaseCorpus::new(),
        bounds,
        &ExploreOptions {
            fail_fast: true,
            threads: 1,
            ..Default::default()
        },
        true,
    );
    assert!(!ff_refusals.is_empty());
    assert!(!ff_run.complete);
    assert_eq!(ff_run.incomplete_reason.as_deref(), Some("fail_fast"));
    assert!(ff_run.nodes < seq_run.nodes);
}

#[test]
fn simulate_embeds_checked_properties_traces_and_explorer_controls() {
    let dir = TempDir::new().expect("tempdir");
    let path = dir.path().join("sim_check.ik.toml");
    fs::write(
        &path,
        r#"
schema_version = 1
machine_id = "sim_check"
machine_version = 1

[facts]
amount = { type = "int", policy = "replace", initial = 0 }

[states]
draft = { initial = true }
approved = {}
paid = { terminal = true }

[invariants.positive_payment]
assert = "state != 'paid' || amount > 0"

[commands.approve]
from = ["draft"]
to = "approved"
args = { amount = "int" }
then = { amount = "$amount" }

[commands.pay]
from = ["approved"]
to = "paid"

[reach]
can_approve = "state == 'approved'"
"#,
    )
    .unwrap();

    let html_out = dir.path().join("explorer.html");
    let (stdout, stderr, code) = ik(&[
        "simulate",
        path.to_str().unwrap(),
        "--check",
        "--out",
        html_out.to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "stdout={stdout}\nstderr={stderr}");
    let html = fs::read_to_string(&html_out).unwrap();
    assert!(html.contains("Checked Properties &amp; Witness Traces"));
    assert!(html.contains("check_results"));
    assert!(html.contains("positive_payment"));
    assert!(html.contains("counterexample"));
    assert!(html.contains("can_approve"));
    assert!(html.contains("Replay Bug"));
    assert!(html.contains("View: Full"));
    assert!(html.contains("btn-prev-step"));

    let plain_out = dir.path().join("no_check.html");
    let (_, plain_err, plain_code) = ik(&[
        "simulate",
        path.to_str().unwrap(),
        "--no-check",
        "--out",
        plain_out.to_str().unwrap(),
    ]);
    assert_eq!(plain_code, 0, "stderr={plain_err}");
    let plain_html = fs::read_to_string(&plain_out).unwrap();
    assert!(!plain_html.contains("\"check_results\":"));
}
