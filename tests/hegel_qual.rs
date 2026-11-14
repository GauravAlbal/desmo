//! VFY-04: pin and qualify hegeltest 0.30.0 as a dev-only engine.

use hegel::generators as gs;
use hegel::TestCase;

#[hegel::test(test_cases = 32)]
fn generation_draws_integers(tc: TestCase) {
    let n = tc.draw(gs::integers::<i32>());
    assert_eq!(n, n);
}

#[hegel::test(test_cases = 32)]
fn stateful_pool_reuses_inserted_value(tc: TestCase) {
    let mut pool = hegel::stateful::pool::<u32>(&tc);
    let value = tc.draw(gs::integers::<u32>());
    pool.add(value);
    let reused = *tc.draw(pool.values_reusable());
    assert_eq!(reused, value);
}

struct Counter {
    value: i64,
}

#[hegel::state_machine]
impl Counter {
    #[rule]
    fn inc(&mut self, tc: TestCase) {
        let delta = tc.draw(gs::integers::<i64>().min_value(1).max_value(3));
        self.value += delta;
    }

    #[invariant]
    fn non_negative(&self, _: TestCase) {
        assert!(self.value >= 0);
    }
}

#[hegel::test(test_cases = 16)]
fn stateful_machine_runs(tc: TestCase) {
    hegel::stateful::run(Counter { value: 0 }, tc);
}

#[test]
fn hegel_is_absent_from_production_deps() {
    let toml = include_str!("../Cargo.toml");
    let deps = toml
        .split("[dev-dependencies]")
        .next()
        .expect("production section");
    assert!(
        !deps.contains("hegeltest"),
        "hegeltest must not appear in production dependencies"
    );
    assert!(toml.contains("hegeltest = \"=0.30.0\""));
}

/// Seeded failing property used only as a child process by [`shrink_emits_replayable_blob`].
#[hegel::test(test_cases = 64, print_blob = true)]
fn seeded_fail_for_blob(tc: TestCase) {
    if std::env::var("HEGEL_QUAL_FAIL").ok().as_deref() != Some("1") {
        return;
    }
    let n = tc.draw(gs::integers::<i32>().min_value(0).max_value(100));
    assert!(n < 50, "expected n < 50, got {n}");
}

#[test]
fn shrink_emits_replayable_blob() {
    let exe = std::env::current_exe().expect("current test exe");
    let output = std::process::Command::new(&exe)
        .env("HEGEL_QUAL_FAIL", "1")
        .env("HEGEL_TEST_CASES", "64")
        .args(["seeded_fail_for_blob", "--nocapture", "--exact"])
        .output()
        .expect("spawn failing Hegel case");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let text = format!("{stdout}\n{stderr}");
    assert!(!output.status.success(), "seeded fail must fail:\n{text}");
    assert!(
        text.contains("reproduce_failure") || text.contains("To reproduce"),
        "missing reproduce blob:\n{text}"
    );
    assert!(
        text.contains("50") || text.contains("n = 50") || text.contains("got 50"),
        "shrink did not reach the boundary witness:\n{text}"
    );
    let replay = std::process::Command::new(&exe)
        .args(["replay_seeded_fail", "--nocapture", "--exact", "--ignored"])
        .output()
        .expect("spawn replay");
    let replay_text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&replay.stdout),
        String::from_utf8_lossy(&replay.stderr)
    );
    assert!(!replay.status.success(), "replay must fail:\n{replay_text}");
    assert!(
        replay_text.contains("got 50"),
        "replay missed the same failure class:\n{replay_text}"
    );
}

#[hegel::test]
#[hegel::reproduce_failure("AAEAAAAACgEAAAAy")]
#[ignore]
fn replay_seeded_fail(tc: TestCase) {
    let n = tc.draw(gs::integers::<i32>().min_value(0).max_value(100));
    assert!(n < 50, "expected n < 50, got {n}");
}
