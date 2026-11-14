//! VFY-08: counterexample artifact and replay classification.

use ik::canonical::canonical_json;
use ik::declarative::{classify_replay, CounterexampleV1, INVALID_NONDETERMINISTIC_COUNTEREXAMPLE};
use std::collections::BTreeMap;

#[test]
fn counterexample_carries_opaque_blob() {
    let cx = CounterexampleV1 {
        schema_version: 1,
        source_id: "CHL-GUARD-BOUNDARY/reject/0".into(),
        machine_id: "verification_attempt".into(),
        spec_hash: "abc".into(),
        failure_class: "n_below_50".into(),
        minimal_steps: vec!["draw".into()],
        minimal_inputs: BTreeMap::from([("n".into(), "50".into())]),
        diagnostic: Some("expected n < 50, got 50".into()),
        engine_version: "hegeltest=0.30.0".into(),
        reproduce_blob: "AAEAAAAACgEAAAAy".into(),
    };
    let json = canonical_json(&cx);
    assert!(json.contains("AAEAAAAACgEAAAAy"));
    assert!(!json.contains("proved"));
    assert!(!json.contains("semantic_authority"));
    assert_eq!(cx.schema_version, 1);
}

#[test]
fn non_reproducing_blob_is_invalid() {
    assert_eq!(
        classify_replay("n_below_50", None),
        INVALID_NONDETERMINISTIC_COUNTEREXAMPLE
    );
    assert_eq!(
        classify_replay("n_below_50", Some("other_class")),
        INVALID_NONDETERMINISTIC_COUNTEREXAMPLE
    );
    assert_eq!(
        classify_replay("n_below_50", Some("n_below_50")),
        "reproduced"
    );
}
