# n13-recall-span

machine: `desk.ik.toml`

## identity
exit=0
```
{"machine_id":"ill_desk_v2_slice","spec_hash":"e539ac24fa921a14fb7e8651c709aea43117b9e44e73c1e3bf34edfe2db43998","version":2}
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
  obligation status: not_examined=6 satisfied=9
  corpus 15a5783f88d4c9413da353d917935fad8f3c9260397b4ed9da7746301efc5b67 (5 cases), exploration complete within the supplied corpus, nodes 3
  excluded: evidence is bounded and relative to the supplied finite corpus; it is not exhaustive proof
  excluded: runtime artifact digest is unavailable: this report is not bound to a specific binary
  excluded: external reducers, hosts, and effect executors are outside this check
  excluded: obligation kinds with no verifier in this lane stay not_examined, not satisfied: OBL-CAP-001, OBL-DETERMINISM-001, OBL-EVENT-001, OBL-REFUSE-001, OBL-REPLAY-001, OBL-TERM-002
  excluded: no change-local adequacy challenges ran (they need --prior; with --prior, none was generated for this change)
```

## scenario `on-time.json`
exit=0
```
all expectations met
```
- genesis: expect=committed outcome=committed verdict=pass facts={"recall_placed": 0}
- checkout: expect=committed outcome=committed verdict=pass facts={"checkout_ms": 1000, "loan_until_ms": 604801000, "recall_placed": 0}
- on-time-checkin: expect=committed outcome=committed verdict=pass facts={"checkout_ms": 1000, "loan_until_ms": 604801000, "recall_placed": 0}

## scenario `recall-and-span.json`
exit=0
```
all expectations met
```
- genesis: expect=committed outcome=committed verdict=pass facts={"recall_placed": 0}
- checkout: expect=committed outcome=committed verdict=pass facts={"checkout_ms": 1000, "loan_until_ms": 604801000, "recall_placed": 0}
- overdue-without-recall: expect=refused:IK_INVARIANT_FAILED outcome=refused verdict=pass facts={"checkout_ms": 1000, "loan_until_ms": 604801000, "recall_placed": 0}
- place-recall: expect=committed outcome=committed verdict=pass facts={"checkout_ms": 1000, "loan_until_ms": 604801000, "recall_placed": 1}
- overdue-with-recall: expect=committed outcome=committed verdict=pass facts={"checkout_ms": 1000, "loan_until_ms": 604801000, "recall_placed": 1}

## scenario `span-lie.json`
exit=0
```
all expectations met
```
- genesis: expect=committed outcome=committed verdict=pass facts={"recall_placed": 0}
- checkout-14d-lie: expect=refused:IK_INVARIANT_FAILED outcome=refused verdict=pass facts={"recall_placed": 0}
