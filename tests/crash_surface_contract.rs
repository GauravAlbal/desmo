//! Compile-surface contract for the crash-arming API (Cargo feature
//! `test-support`, NON-default).
//!
//! The crash-injection seam — `CrashHook`, `Store::open_with_crash_hook`, the
//! `Store.crash_hook` field, `fire_crash`, and every `crash.*` fire point in
//! `src/store.rs` — is gated behind `#[cfg(feature = "test-support")]`. Contract:
//!
//! - a normal `cargo build` / `cargo test` (default features) compiles the crate
//!   with NO crash-arming API: nothing in the production binary can abort a
//!   process from ambient state;
//!
//! `cargo test --all-features` compiles the seam and runs the real process-crash
//! matrix in `tests/crash_matrix.rs` (itself feature-gated).
//!
//! The gate's observable consequence is the shipped artifact: in the default
//! build the compiled `ik` binary contains none of the crash failpoint literals.
//! This file compiles in BOTH configurations. Under default features it proves
//! the seam is compiled out of the actual binary; under `--all-features` it is
//! compiled away and `crash_matrix.rs` (feature-gated) proves the seam compiles
//! and fires.
//!
//! The atomic create-with-command operation (`create_aggregate_with_command`)
//! emits the same `{kind}`-formatted boundaries with `kind="genesis"` (its
//! genesis persist) and `kind="admit"` (its initial-command persist); both are
//! formatted at runtime from the same three `crash.{{kind}}.*` literals listed
//! below, so no additional literal escapes the gate.

/// In the default build the crash-arming seam must be compiled out: the shipped
/// `ik` binary must not contain any crash failpoint literal. Removing the
/// `#[cfg(feature = "test-support")]` gate from a fire point in `src/store.rs`
/// leaks its literal into the default binary and fails this test (and thereby
/// the default build). The four literals below are the exact strings emitted by
/// `fire_crash`; the `{kind}` variants (genesis/command/record) are formatted at
/// runtime from the same three literals.
#[cfg(not(feature = "test-support"))]
#[test]
fn default_build_binary_contains_no_crash_arming() {
    let bin = env!("CARGO_BIN_EXE_ik");
    let bytes = std::fs::read(bin).expect("read default-build ik binary");
    for literal in [
        "crash.genesis.after_aggregates_insert",
        "crash.{kind}.after_journal",
        "crash.{kind}.after_receipts",
        "crash.{kind}.after_projection",
    ] {
        assert!(
            !bytes
                .windows(literal.len())
                .any(|w| w == literal.as_bytes()),
            "default-build ik binary must not contain crash failpoint `{literal}`; \
             the `test-support` gate leaked into the default build"
        );
    }
}
