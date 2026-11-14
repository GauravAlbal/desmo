# n14-fill-arith

machine: `rx.ik.toml`

## identity
exit=0
```
{"machine_id":"rx_fill_slice","spec_hash":"2a8754e2c8b7fb889b966fa120fffecdb38ac1eba22cf7332e0cc6886b9081f9","version":1}
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

RELEASE POLICY (strict; separate from developer ACCEPT)
  verdict release_pass
  obligation status: not_examined=5 satisfied=6
  corpus 9092a3fa956b53e06339ce75031b0f5635c5c71ab485f2f1441f64cf3742b06f (5 cases), exploration complete within the supplied corpus, nodes 3
  excluded: evidence is bounded and relative to the supplied finite corpus; it is not exhaustive proof
  excluded: runtime artifact digest is unavailable: this report is not bound to a specific binary
  excluded: external reducers, hosts, and effect executors are outside this check
  excluded: obligation kinds with no verifier in this lane stay not_examined, not satisfied: OBL-CAP-001, OBL-DETERMINISM-001, OBL-EVENT-001, OBL-REFUSE-001, OBL-REPLAY-001
  excluded: no change-local adequacy challenges ran (they need --prior; with --prior, none was generated for this change)
```

## scenario `amount-lie.json`
exit=0
```
all expectations met
```
- genesis: expect=committed outcome=committed verdict=pass facts={}
- intake: expect=committed outcome=committed verdict=pass facts={"hold_ms": 5000, "remaining_qty": 10}
- fill-amount-lie: expect=refused:IK_INVARIANT_FAILED outcome=refused verdict=pass facts={"hold_ms": 5000, "remaining_qty": 10}

## scenario `deadline-lie.json`
exit=0
```
all expectations met
```
- genesis: expect=committed outcome=committed verdict=pass facts={}
- intake: expect=committed outcome=committed verdict=pass facts={"hold_ms": 5000, "remaining_qty": 10}
- fill-deadline-lie: expect=refused:IK_INVARIANT_FAILED outcome=refused verdict=pass facts={"hold_ms": 5000, "remaining_qty": 10}

## scenario `honest-fill.json`
exit=0
```
all expectations met
```
- genesis: expect=committed outcome=committed verdict=pass facts={}
- intake: expect=committed outcome=committed verdict=pass facts={"hold_ms": 5000, "remaining_qty": 10}
- fill: expect=committed outcome=committed verdict=pass facts={"amount_cents": 700, "held_units": 4, "hold_ms": 5000, "pickup_deadline_ms": 6000, "remaining_qty": 6}

## scenario `remain-lie.json`
exit=0
```
all expectations met
```
- genesis: expect=committed outcome=committed verdict=pass facts={}
- intake: expect=committed outcome=committed verdict=pass facts={"hold_ms": 5000, "remaining_qty": 10}
- fill-remain-lie: expect=refused:IK_INVARIANT_FAILED outcome=refused verdict=pass facts={"hold_ms": 5000, "remaining_qty": 10}
