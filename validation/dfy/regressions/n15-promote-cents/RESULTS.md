# n15-promote-cents

machine: `listing.ik.toml`

## identity
exit=0
```
{"machine_id":"listing_promote_slice","spec_hash":"ae34e1a70583f21fddb3fb8c4e12d5cf297fa55fe372972ffcb659fb13d71ff2","version":2}
```

## check
exit=0
```
VERDICT
  ACCEPT
ASSURANCE
  class bounded_checked
  exploration complete_within_corpus
  ACCEPT is corpus-relative, not a proof

IMPOSSIBLE SUCCESS
  outcome/deposit/0/executable
    class ordered_outcome
    discharged false (obligation retained)
    reason successful commit is impossible: guard_false
    proving_source under_contract
    basis guard_false
    scope arm=0

RELEASE POLICY (strict; separate from developer ACCEPT)
  verdict release_pass
  obligation status: not_examined=5 satisfied=16
  corpus 1e540f88124083124383382479ad89e6d74c986104aba51ec17158b953a9bec3 (7 cases), exploration complete within the supplied corpus, nodes 7
  excluded: evidence is bounded and relative to the supplied finite corpus; it is not exhaustive proof
  excluded: runtime artifact digest is unavailable: this report is not bound to a specific binary
  excluded: external reducers, hosts, and effect executors are outside this check
  excluded: obligation kinds with no verifier in this lane stay not_examined, not satisfied: OBL-CAP-001, OBL-DETERMINISM-001, OBL-EVENT-001, OBL-REFUSE-001, OBL-REPLAY-001
  excluded: no change-local adequacy challenges ran (they need --prior; with --prior, none was generated for this change)
```

## scenario `cents-lie.json`
exit=0
```
all expectations met
```
- genesis: expect=committed outcome=committed verdict=pass facts={"accepted_ms": 0, "deposit_cents": 0, "deposit_deadline_ms": 0}
- open: expect=committed outcome=committed verdict=pass facts={"accepted_ms": 0, "amount": 10000, "deposit_cents": 0, "deposit_deadline_ms": 0, "deposit_window_ms": 5000}
- accept-cents-lie: expect=refused:IK_INVARIANT_FAILED outcome=refused verdict=pass facts={"accepted_ms": 0, "amount": 10000, "deposit_cents": 0, "deposit_deadline_ms": 0, "deposit_window_ms": 5000}

## scenario `promote-reconcile.json`
exit=0
```
all expectations met
```
- genesis: expect=committed outcome=committed verdict=pass facts={"accepted_ms": 0, "deposit_cents": 0, "deposit_deadline_ms": 0}
- open: expect=committed outcome=committed verdict=pass facts={"accepted_ms": 0, "amount": 10000, "deposit_cents": 0, "deposit_deadline_ms": 0, "deposit_window_ms": 5000}
- accept: expect=committed outcome=committed verdict=pass facts={"accepted_ms": 1000, "amount": 10000, "deposit_cents": 200, "deposit_deadline_ms": 6000, "deposit_window_ms": 5000}
- deposit: expect=committed outcome=committed verdict=pass facts={"accepted_ms": 1000, "amount": 10000, "deposit_cents": 200, "deposit_deadline_ms": 6000, "deposit_id": "dep-1", "deposit_window_ms": 5000}
- fallthrough: expect=committed outcome=committed verdict=pass facts={"accepted_ms": 1000, "amount": 10000, "deposit_cents": 200, "deposit_deadline_ms": 6000, "deposit_id": "dep-1", "deposit_window_ms": 5000}
- promote: expect=committed outcome=committed verdict=pass facts={"accepted_ms": 0, "amount": 10000, "deposit_cents": 0, "deposit_deadline_ms": 0, "deposit_id": "", "deposit_window_ms": 5000}
- reconcile: expect=committed outcome=committed verdict=pass facts={"accepted_ms": 0, "amount": 10000, "deposit_cents": 0, "deposit_deadline_ms": 0, "deposit_id": "", "deposit_window_ms": 5000}
after promote facts: {"accepted_ms": 0, "amount": 10000, "deposit_cents": 0, "deposit_deadline_ms": 0, "deposit_id": "", "deposit_window_ms": 5000}
reconcile after promote committed (not leftover-deposit)
