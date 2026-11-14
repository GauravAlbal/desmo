# Agent interface fixture

Technical command outputs for the three journeys. Not the human readout.

Packed binary version line:

~~~~text
invariant-kernel 0.5.0 protocol 1.0
~~~~

## BUILD

### Identity

`ik identity fixtures/tagged_item.ik.toml`

~~~~text
{"machine_id":"tagged_item","spec_hash":"f0bef35d059f09af32614c758cbfb5f66ac707bf92c03240dd96b729cb3758d6","version":1}
~~~~


### Laws inventory

`ik laws fixtures/tagged_item.ik.toml`

~~~~text
{"advisory":true,"commands":[{"enforcement":"runtime","name":"tag"}],"laws":[],"machine_id":"tagged_item","spec_hash":"f0bef35d059f09af32614c758cbfb5f66ac707bf92c03240dd96b729cb3758d6","unguarded_arguments":[{"advice":"no guard reads this argument; a rule on its value (non-empty, bounded, in a set, related to a fact) is enforced only by host code","argument":"label","command":"tag","type":"string"}],"version":1}
~~~~


### Developer check

`ik check fixtures/tagged_item.ik.toml --corpus fixtures/tagged_item.ik.cases.json --profile fixtures/tagged_item.ik.checks.toml`

~~~~text
VERDICT
  ACCEPT
ASSURANCE
  class bounded_checked
  exploration complete_within_corpus
  ACCEPT is corpus-relative, not a proof

RELEASE POLICY (strict; separate from developer ACCEPT)
  verdict release_pass
  obligation status: not_examined=6 satisfied=4
  corpus 6d607ce7a815e81b109d2961ec78e793117505e289d0c46ba321deca0e72d93e (6 cases), exploration complete within the supplied corpus, nodes 1
  excluded: evidence is bounded and relative to the supplied finite corpus; it is not exhaustive proof
  excluded: runtime artifact digest is unavailable: this report is not bound to a specific binary
  excluded: external reducers, hosts, and effect executors are outside this check
  excluded: obligation kinds with no verifier in this lane stay not_examined, not satisfied: OBL-CAP-001, OBL-DETERMINISM-001, OBL-EVENT-001, OBL-REFUSE-001, OBL-REPLAY-001, OBL-TERM-002
  excluded: no change-local adequacy challenges ran (they need --prior; with --prior, none was generated for this change)
PROFILE
  profile_digest edb45f69c3d75931eba54eb8f71e330e5f9ad122e8d1f1f7e19a80681a2432ed
  machine_spec_hash f0bef35d059f09af32614c758cbfb5f66ac707bf92c03240dd96b729cb3758d6
  corpus_digest 6d607ce7a815e81b109d2961ec78e793117505e289d0c46ba321deca0e72d93e
  bounds max_nodes=64 max_depth=8
  search_complete true
  goal tagged witness_found required=true content_digest=fcd46959cc2cd898a3ad9c2d2c5f903e1b4b9043c5488eca7a72e46c6b8723d8
    witness: tag[valid]
~~~~


### Scenario run

`ik scenario run fixtures/tagged_item.ik.toml fixtures/scenarios/honest-tag.json --out out/sc-build`

~~~~text
all expectations met
~~~~


### Report

`ik report out/sc-build out/check-build.json`

~~~~text
ik.report.v1
finding out/sc-build#item-1/genesis
  source: ik.scenario.result.v1
  status: committed
  scope: observed runtime trace; the command committed
  evidence: observed_trace
  completeness: observed
  coordinate: state=idle
  after: {"effect":{"status":"none"},"facts":{},"generation":0,"state":"idle"}
  attempted: {"aggregate":"item-1"}
  committed: committed
  reproduction: observed
  artifact: out/sc-build/SHA256SUMS
  artifact: out/sc-build/machine.ik.toml
  artifact: out/sc-build/result.json
  artifact: out/sc-build/scenario.json
  artifact: out/sc-build/trace.json
  next: promote as regression or continue
finding out/sc-build#item-1/tag
  source: ik.scenario.result.v1
  status: committed
  scope: observed runtime trace; the command committed
  evidence: observed_trace
  completeness: observed
  coordinate: command=tag state=tagged
  before: {"effect":{"status":"none"},"facts":{},"generation":0,"state":"idle"}
  after: {"effect":{"status":"none"},"facts":{"item_id":"item_1","label":"AMBER"},"generation":1,"state":"tagged"}
  attempted: tag {"id":"item_1","label":"AMBER"}
  committed: committed
  reproduction: observed
  artifact: out/sc-build/SHA256SUMS
  artifact: out/sc-build/machine.ik.toml
  artifact: out/sc-build/result.json
  artifact: out/sc-build/scenario.json
  artifact: out/sc-build/trace.json
  next: promote as regression or continue
finding out/check-build.json#tagged
  source: ik.check.profile.v1
  status: witness_found
  scope: bounded search over the supplied corpus; smallest found within the enumerated search
  evidence: bounded_paths
  completeness: complete
  coordinate: command=tag
  attempted: reach_terminal tagged
  committed: not a runtime commit
  reproduction: not_attempted
  next: promote as regression or continue
~~~~


## CHANGE

### Identity of the previous definition

`ik identity fixtures/loan_checkout.ik.toml`

~~~~text
{"machine_id":"loan_checkout","spec_hash":"6c08d4be51065bc9fd184c78009f7ef1f3b5c40f82e1d2d9d46e226cabaa15d8","version":1}
~~~~


### Identity of the new definition

`ik identity fixtures/loan_v2.ik.toml`

~~~~text
{"machine_id":"loan_checkout","spec_hash":"d38ced2adcf44ad5af742c25595f2148e071be1af1f8b5be4ef8395bfcef86ee","version":2}
~~~~


### Developer check with prior

`ik check fixtures/loan_v2.ik.toml --prior fixtures/loan_checkout.ik.toml --corpus fixtures/loan_v2.ik.cases.json --profile fixtures/loan_checkout.ik.checks.toml`

~~~~text
NEW SEMANTIC DEBT

command/extend/from/on_loan/executable
  executable witness     FOUND

outcome/extend/0/executable
  executable witness     FOUND   checkout,extend

CHANGE IMPACT
  new machine version 2 (prior version 1)
  existing instances keep the version they were created under
  commands added: extend
  commands removed: none
  states added: none
  states removed: none
  existing instances stay on their version; moving one to version 2 in place is not supported

VERDICT
  ACCEPT
ASSURANCE
  class bounded_checked
  exploration complete_within_corpus
  ACCEPT is corpus-relative, not a proof

RELEASE POLICY (strict; separate from developer ACCEPT)
  verdict release_pass
  obligation status: not_examined=6 satisfied=11
  corpus 453712ecad5b91c72b02a8d9c8d3638a08ee82a62ed2336688524a0e721e752f (5 cases), exploration complete within the supplied corpus, nodes 5
  excluded: evidence is bounded and relative to the supplied finite corpus; it is not exhaustive proof
  excluded: runtime artifact digest is unavailable: this report is not bound to a specific binary
  excluded: external reducers, hosts, and effect executors are outside this check
  excluded: obligation kinds with no verifier in this lane stay not_examined, not satisfied: OBL-CAP-001, OBL-DETERMINISM-001, OBL-EVENT-001, OBL-REFUSE-001, OBL-REPLAY-001, OBL-TERM-002
  excluded: no change-local adequacy challenges ran (they need --prior; with --prior, none was generated for this change)
PROFILE
  profile_digest cdabbfae9b0f19c2270c33f038686b1b469d3533f3fe6a8f36840b7263ef4f7b
  machine_spec_hash d38ced2adcf44ad5af742c25595f2148e071be1af1f8b5be4ef8395bfcef86ee
  corpus_digest 453712ecad5b91c72b02a8d9c8d3638a08ee82a62ed2336688524a0e721e752f
  bounds max_nodes=64 max_depth=8
  search_complete true
  goal returned witness_found required=true content_digest=0e7917dc0aa3e40bc90768d5668d605a384e5e43ba5e034d13ed5902adad47e0
    witness: checkout[honest] checkin[on_time]
  baseline
  goal returned candidate=witness_found prior=witness_found
~~~~


### Report of that check

`ik report out/check-change.json`

~~~~text
ik.report.v1
finding out/check-change.json#definition
  source: ik.check.profile.v1
  status: declared
  scope: new in version 2: command extend; removed: none
  evidence: definition
  completeness: declared
  attempted: prior-vs-new comparison
  committed: not a runtime commit
  reproduction: not_attempted
  next: declared only; does not establish execution
finding out/check-change.json#returned
  source: ik.check.profile.v1
  status: witness_found
  scope: bounded search over the supplied corpus; smallest found within the enumerated search
  evidence: bounded_paths
  completeness: complete
  coordinate: command=checkout
  attempted: reach_terminal returned
  committed: not a runtime commit
  reproduction: not_attempted
  next: promote as regression or continue
~~~~


### Existing-instance continuity scenario

`ik scenario run fixtures/loan_v2.ik.toml fixtures/scenarios/existing-loan.json --out out/sc-change`

~~~~text
all expectations met
~~~~


### Report of that scenario

`ik report out/sc-change`

~~~~text
ik.report.v1
finding out/sc-change#loan-1/under_prior
  source: ik.scenario.result.v1
  status: committed
  scope: instances created under version 1 keep version 1
  evidence: continuity_observation
  completeness: observed
  coordinate: state=idle
  after: {"effect":{"status":"none"},"facts":{"overdue":0,"recall_placed":0},"generation":0,"state":"idle"}
  attempted: {"aggregate":"loan-1","path":"../loan_checkout.ik.toml"}
  committed: committed
  reproduction: observed
  artifact: out/sc-change/SHA256SUMS
  artifact: out/sc-change/machine.ik.toml
  artifact: out/sc-change/prior_machine.ik.toml
  artifact: out/sc-change/result.json
  artifact: out/sc-change/scenario.json
  artifact: out/sc-change/trace.json
  next: existing instances stay on their version; moving one to the new version in place is not supported
finding out/sc-change#loan-1/checkout
  source: ik.scenario.result.v1
  status: committed
  scope: instances created under version 1 keep version 1; checkout still works on them (presented version 1)
  evidence: continuity_observation
  completeness: observed
  coordinate: command=checkout state=on_loan
  before: {"effect":{"status":"none"},"facts":{"overdue":0,"recall_placed":0},"generation":0,"state":"idle"}
  after: {"effect":{"status":"none"},"facts":{"checkout_ms":1000,"deposit_id":"dep-1","loan_until_ms":1209601000,"overdue":0,"recall_placed":0},"generation":1,"state":"on_loan"}
  attempted: checkout {"checkout_ms":1000,"deposit_id":"dep-1","loan_until_ms":1209601000}
  committed: committed
  reproduction: observed
  artifact: out/sc-change/SHA256SUMS
  artifact: out/sc-change/machine.ik.toml
  artifact: out/sc-change/prior_machine.ik.toml
  artifact: out/sc-change/result.json
  artifact: out/sc-change/scenario.json
  artifact: out/sc-change/trace.json
  next: existing instances stay on their version; moving one to the new version in place is not supported
finding out/sc-change#loan-1/extend
  source: ik.scenario.result.v1
  status: refused
  scope: instances created under version 1 keep version 1; extend is refused on them with IK_MACHINE_MISMATCH (presented version 2)
  evidence: continuity_observation
  completeness: observed
  coordinate: command=extend state=on_loan
  before: {"effect":{"status":"none"},"facts":{"checkout_ms":1000,"deposit_id":"dep-1","loan_until_ms":1209601000,"overdue":0,"recall_placed":0},"generation":1,"state":"on_loan"}
  after: {"effect":{"status":"none"},"facts":{"checkout_ms":1000,"deposit_id":"dep-1","loan_until_ms":1209601000,"overdue":0,"recall_placed":0},"generation":1,"state":"on_loan"}
  attempted: extend {"new_until_ms":1209601001}
  committed: refused
  reproduction: observed
  artifact: out/sc-change/SHA256SUMS
  artifact: out/sc-change/machine.ik.toml
  artifact: out/sc-change/prior_machine.ik.toml
  artifact: out/sc-change/result.json
  artifact: out/sc-change/scenario.json
  artifact: out/sc-change/trace.json
  next: existing instances stay on their version; moving one to version 2 in place is not supported
finding out/sc-change#loan-1/recall
  source: ik.scenario.result.v1
  status: committed
  scope: instances created under version 1 keep version 1; recall still works on them (presented version 1)
  evidence: continuity_observation
  completeness: observed
  coordinate: command=recall state=on_loan
  before: {"effect":{"status":"none"},"facts":{"checkout_ms":1000,"deposit_id":"dep-1","loan_until_ms":1209601000,"overdue":0,"recall_placed":0},"generation":1,"state":"on_loan"}
  after: {"effect":{"status":"none"},"facts":{"checkout_ms":1000,"deposit_id":"dep-1","loan_until_ms":1209601000,"overdue":0,"recall_placed":1},"generation":2,"state":"on_loan"}
  attempted: recall {}
  committed: committed
  reproduction: observed
  artifact: out/sc-change/SHA256SUMS
  artifact: out/sc-change/machine.ik.toml
  artifact: out/sc-change/prior_machine.ik.toml
  artifact: out/sc-change/result.json
  artifact: out/sc-change/scenario.json
  artifact: out/sc-change/trace.json
  next: existing instances stay on their version; moving one to the new version in place is not supported
~~~~


### Live store operations on an existing loan (product JSON as printed)

`ik --store loan.sqlite --machine <definition> create|command|show`

~~~~text
create:
{"ok":true,"protocol":{"major":1,"minor":0},"request_id":"lc1","result":{"aggregate_id":"loan-1","capability":{"generation":0,"nonce":"<hex>"},"effect":{"status":"none"},"generation":0,"mutation":true,"receipt":{"aggregate_id":"loan-1","effect":{"type":"none"},"event_hash":"<hex>","generation":0,"machine_id":"loan_checkout","machine_spec_hash":"6c08d4be51065bc9fd184c78009f7ef1f3b5c40f82e1d2d9d46e226cabaa15d8","machine_version":1,"receipt_id":"<hex>","sequence":0,"state_digest":"<hex>"},"state":{"overdue":0,"recall_placed":0,"state":"idle"},"terminal":false},"runtime_version":"0.5.0"}
checkout:
{"ok":true,"protocol":{"major":1,"minor":0},"request_id":"co1","result":{"aggregate_id":"loan-1","capability":{"generation":1,"nonce":"<hex>"},"effect":{"status":"none"},"generation":1,"mutation":true,"receipt":{"aggregate_id":"loan-1","effect":{"type":"none"},"event_hash":"<hex>","generation":1,"machine_id":"loan_checkout","machine_spec_hash":"6c08d4be51065bc9fd184c78009f7ef1f3b5c40f82e1d2d9d46e226cabaa15d8","machine_version":1,"receipt_id":"<hex>","sequence":1,"state_digest":"<hex>"},"state":{"checkout_ms":1000,"deposit_id":"dep-1","loan_until_ms":1209601000,"overdue":0,"recall_placed":0,"state":"on_loan"},"terminal":false},"runtime_version":"0.5.0"}
show:
{"ok":true,"protocol":{"major":1,"minor":0},"request_id":"sh1","result":{"aggregate_id":"loan-1","capability":{"generation":1,"nonce":"<hex>"},"effect":{"status":"none"},"generation":1,"last_event_hash":"<hex>","last_receipt":{"aggregate_id":"loan-1","effect":{"type":"none"},"event_hash":"<hex>","generation":1,"machine_id":"loan_checkout","machine_spec_hash":"6c08d4be51065bc9fd184c78009f7ef1f3b5c40f82e1d2d9d46e226cabaa15d8","machine_version":1,"receipt_id":"<hex>","sequence":1,"state_digest":"<hex>"},"machine":{"machine_id":"loan_checkout","spec_hash":"6c08d4be51065bc9fd184c78009f7ef1f3b5c40f82e1d2d9d46e226cabaa15d8","version":1},"sequence":1,"state":{"checkout_ms":1000,"deposit_id":"dep-1","loan_until_ms":1209601000,"overdue":0,"recall_placed":0,"state":"on_loan"},"terminal":false},"runtime_version":"0.5.0"}
extend under the version-2 definition:
{"diagnostic":{"code":"IK_MACHINE_MISMATCH","message":"presented machine identity does not match the aggregate's recorded identity"},"ok":false,"protocol":{"major":1,"minor":0},"request_id":"ext1","runtime_version":"0.5.0"}
recall under the version-1 definition:
{"ok":true,"protocol":{"major":1,"minor":0},"request_id":"rec1","result":{"aggregate_id":"loan-1","capability":{"generation":2,"nonce":"<hex>"},"effect":{"status":"none"},"generation":2,"mutation":true,"receipt":{"aggregate_id":"loan-1","effect":{"type":"none"},"event_hash":"<hex>","generation":2,"machine_id":"loan_checkout","machine_spec_hash":"6c08d4be51065bc9fd184c78009f7ef1f3b5c40f82e1d2d9d46e226cabaa15d8","machine_version":1,"receipt_id":"<hex>","sequence":2,"state_digest":"<hex>"},"state":{"checkout_ms":1000,"deposit_id":"dep-1","loan_until_ms":1209601000,"overdue":0,"recall_placed":1,"state":"on_loan"},"terminal":false},"runtime_version":"0.5.0"}
~~~~


## DIAGNOSE / RECOVER

### Scenario run

`ik scenario run fixtures/tagged_item.ik.toml fixtures/scenarios/lost-reply.json --out out/sc-lost`

~~~~text
all expectations met
~~~~


### Report

`ik report out/sc-lost`

~~~~text
ik.report.v1
finding out/sc-lost#item-lost/genesis
  source: ik.scenario.result.v1
  status: committed
  scope: observed runtime trace; the command committed
  evidence: observed_trace
  completeness: observed
  coordinate: state=idle
  after: {"effect":{"status":"none"},"facts":{},"generation":0,"state":"idle"}
  attempted: {"aggregate":"item-lost"}
  committed: committed
  reproduction: observed
  artifact: out/sc-lost/SHA256SUMS
  artifact: out/sc-lost/machine.ik.toml
  artifact: out/sc-lost/result.json
  artifact: out/sc-lost/scenario.json
  artifact: out/sc-lost/trace.json
  next: promote as regression or continue
finding out/sc-lost#item-lost/tag
  source: ik.scenario.result.v1
  status: committed
  scope: observed runtime trace; the command committed
  evidence: observed_trace
  completeness: observed
  coordinate: command=tag state=tagged
  before: {"effect":{"status":"none"},"facts":{},"generation":0,"state":"idle"}
  after: {"effect":{"status":"none"},"facts":{"item_id":"item_1","label":"AMBER"},"generation":1,"state":"tagged"}
  attempted: tag {"id":"item_1","label":"AMBER"}
  committed: committed
  reproduction: observed
  artifact: out/sc-lost/SHA256SUMS
  artifact: out/sc-lost/machine.ik.toml
  artifact: out/sc-lost/result.json
  artifact: out/sc-lost/scenario.json
  artifact: out/sc-lost/trace.json
  next: promote as regression or continue
finding out/sc-lost#item-lost/lost
  source: ik.scenario.result.v1
  status: reply_dropped
  scope: established: the reply to the earlier attempt was not delivered. uncertain: this dropped-reply record does not by itself confirm the commit
  evidence: observed_trace
  completeness: observed
  coordinate: state=tagged
  before: {"effect":{"status":"none"},"facts":{"item_id":"item_1","label":"AMBER"},"generation":1,"state":"tagged"}
  after: {"effect":{"status":"none"},"facts":{"item_id":"item_1","label":"AMBER"},"generation":1,"state":"tagged"}
  attempted: {"of":"tag"}
  committed: the reply was discarded; this step did not commit
  reproduction: observed
  artifact: out/sc-lost/SHA256SUMS
  artifact: out/sc-lost/machine.ik.toml
  artifact: out/sc-lost/result.json
  artifact: out/sc-lost/scenario.json
  artifact: out/sc-lost/trace.json
  next: look up the earlier operation; do not resend from the dropped reply
finding out/sc-lost#item-lost/found
  source: ik.scenario.result.v1
  status: committed_exact
  scope: established: the earlier operation committed; this lookup did not commit again. uncertain: whether any handle from this lookup is current
  evidence: observed_trace
  completeness: observed
  coordinate: state=tagged
  before: {"effect":{"status":"none"},"facts":{"item_id":"item_1","label":"AMBER"},"generation":1,"state":"tagged"}
  after: {"effect":{"status":"none"},"facts":{"item_id":"item_1","label":"AMBER"},"generation":1,"state":"tagged"}
  attempted: {"operation_id":"op-tag"}
  committed: earlier operation: committed (confirmed by lookup); this lookup did not commit again
  reproduction: observed
  artifact: out/sc-lost/SHA256SUMS
  artifact: out/sc-lost/machine.ik.toml
  artifact: out/sc-lost/result.json
  artifact: out/sc-lost/scenario.json
  artifact: out/sc-lost/trace.json
  next: do not resend; the earlier result stands
finding out/sc-lost#item-lost/again
  source: ik.scenario.result.v1
  status: historical
  scope: established: the earlier operation already committed; this attempt did not commit again. uncertain: whether any returned handle is current
  evidence: observed_trace
  completeness: observed
  coordinate: state=tagged
  before: {"effect":{"status":"none"},"facts":{"item_id":"item_1","label":"AMBER"},"generation":1,"state":"tagged"}
  after: {"effect":{"status":"none"},"facts":{"item_id":"item_1","label":"AMBER"},"generation":1,"state":"tagged"}
  attempted: {"of":"tag","operation_id":"op-tag"}
  committed: earlier operation: already committed (historical); this attempt did not commit again
  reproduction: observed
  artifact: out/sc-lost/SHA256SUMS
  artifact: out/sc-lost/machine.ik.toml
  artifact: out/sc-lost/result.json
  artifact: out/sc-lost/scenario.json
  artifact: out/sc-lost/trace.json
  next: do not treat this as a new commit or a current handle
finding out/sc-lost#item-lost/restart
  source: ik.scenario.result.v1
  status: reopened
  scope: observed runtime trace; outcome reopened
  evidence: observed_trace
  completeness: observed
  coordinate: state=tagged
  before: {"effect":{"status":"none"},"facts":{"item_id":"item_1","label":"AMBER"},"generation":1,"state":"tagged"}
  after: {"effect":{"status":"none"},"facts":{"item_id":"item_1","label":"AMBER"},"generation":1,"state":"tagged"}
  attempted: {}
  committed: not committed
  reproduction: observed
  artifact: out/sc-lost/SHA256SUMS
  artifact: out/sc-lost/machine.ik.toml
  artifact: out/sc-lost/result.json
  artifact: out/sc-lost/scenario.json
  artifact: out/sc-lost/trace.json
  next: inspect status reopened
~~~~

