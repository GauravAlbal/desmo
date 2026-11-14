# Human readout sheet (~10 minutes)

Packed `ik` 0.5.0, protocol 1.0. Read codes, rule ids, and status fields.
Do not judge by message wording. Answer the questions; then the key.

Machine under test: `tagged_item` (identifier grammar on `tag.id`) and
`loan_checkout` v1/v2 (old instances keep v1 identity).

```text
Brief -> ToolReady -> Drafted -> Checked -> ScenarioVerified -> Operated
                                      \-> FailureObserved -> Diagnosed -> Recovered
ChangeRequested -> Changed -> Versioned  (old aggregate stays on v1)
```

## Journey A — happy BUILD

`ik --version` → `invariant-kernel 0.5.0 protocol 1.0`

`ik identity fixtures/tagged_item.ik.toml`

```json
{"machine_id":"tagged_item","version":1,"spec_hash":"f0bef35d059f09af32614c758cbfb5f66ac707bf92c03240dd96b729cb3758d6"}
```

`ik check fixtures/tagged_item.ik.toml --corpus fixtures/tagged_item.ik.cases.json --profile fixtures/tagged_item.ik.checks.toml --json`

```json
{"judgment":{"reasons":[],"verdict":"accept"},
 "profile":{"goals":[{"id":"tagged","status":"witness_found"}]}}
```

`ik --store $TMP/tagged.sqlite --machine fixtures/tagged_item.ik.toml` create, then command `tag` `{id:item_1,label:AMBER}`:

```json
{"ok":true,"result":{"generation":1,"mutation":true,"state":{"state":"tagged"},"terminal":true}}
```

A1. After create, what state is the aggregate in, and which field proves the later `tag` committed rather than being a read?
A2. What does `judgment.verdict=accept` plus `status=witness_found` allow you to claim, and what does it not allow?
A3. `ik check --plan --json` also exits 0 with schema `ik.check.plan.v1`. Why is that not the same as A2?

## Journey B — unhappy refused command (`text_in`)

Idle aggregate, current capability, `tag` with `id=""`:

```json
{"ok":false,"diagnostic":{"code":"IK_INVARIANT_FAILED","rule":"tag.when"},
 "protocol":{"major":1,"minor":0},"runtime_version":"0.5.0"}
```

Same capability is still current (refusal does not advance generation).
A second `tag` from terminal `tagged` (current cap) is different:

```json
{"ok":false,"diagnostic":{"code":"IK_ILLEGAL_STATE","rule":"tag"}}
```

A copied generation-0 capability after the successful tag:

```json
{"ok":false,"diagnostic":{"code":"IK_CAPABILITY_STALE","rule":"capability.generation"}}
```

B1. What state is the machine in after the empty-`id` command, and why was it refused? Name the code and the rule id.
B2. How do you tell a grammar/`when` miss from "this command is not legal in this state" from "I reused an old handle"?
B3. What do you do next for the empty `id`? (Do not refresh the capability. Do not edit the store.)

## Journey C — change on an old instance

v1 aggregate left `on_loan`. v2 adds `extend`. Check of v2 with `--prior` v1: `judgment.verdict=accept`, baseline `candidate=witness_found` and `prior=witness_found`. Then v2 `extend` on the old aggregate:

```json
{"ok":false,"diagnostic":{"code":"IK_MACHINE_MISMATCH"}}
```

v1 `recall` on the same aggregate:

```json
{"ok":true,"result":{"mutation":true,"state":{"state":"on_loan","recall_placed":1}}}
```

C1. What identity does the old aggregate still have, and which code is the kernel's refusal of the v2 command?
C2. Is `IK_MACHINE_MISMATCH` a continuity break, or the expected freeze of a v1 instance?
C3. What would you do next to offer `extend` to *new* loans without rewriting the stored v1 row?

## Agent instructions (`skills/ikaros/build.md`)

D1. When do you write `text_in($arg, 'A-Za-z0-9._-', 1, 64)`?
D2. `ik laws` listed `unguarded_arguments` for `tag.label`. Do you add a dummy guard to clear the list?
D3. `ik check` with no corpus returned `judgment.verdict=refused` with `judgment.status=missing_evidence` and `next_actions` starting `ik corpus init`. Is that a machine defect?

## Answer key

A1. Create: `state=idle`, generation 0. `tag` committed because `ok=true`, `mutation=true`, `generation=1`, `state.state=tagged` (a `show` agrees; a read would not set `mutation=true`).
A2. Bounded, corpus-relative developer accept: this search witnessed required goal `tagged`. Not unreachability, not a production commit, not a proof over unsupplied cases.
A3. `--plan` does not explore. Exit 0 is the plan document, not a witness and not a release pass.

B1. Still `idle` (no commit). Refused: `IK_INVARIANT_FAILED` / `tag.when` (`text_in` on `id` failed).
B2. `tag.when` + `IK_INVARIANT_FAILED` = guard miss. `IK_ILLEGAL_STATE` / `tag` = not legal from `tagged`. `IK_CAPABILITY_STALE` / `capability.generation` = old handle.
B3. Send a valid `id` (1..64 of `A-Za-z0-9._-`) with the **current** capability. Do not retry the empty id under a new handle to "make it work."

C1. Recorded v1 identity (`machine_id=loan_checkout`, version 1). Kernel code: `IK_MACHINE_MISMATCH`. (There is no kernel token `legacy_request`.)
C2. Expected freeze: old aggregates keep the identity they were created under. A continuity break would be `corrupt` / a v1 command that used to work now refusing.
C3. Keep the v1 definition loadable; route existing aggregates by `show` owner identity; create new aggregates under v2.

D1. When the contract is a length-bounded character-set identifier grammar. Encode it as a `when` guard on every argument of that type; prove it with a direct runtime refusal (this sheet's empty `id`).
D2. No. Opaque `label` has no extra contract; do not add a meaningless guard to clear the inventory.
D3. No. Missing evidence, not a defect. Add a witness corpus (happy path plus the refusals you expect) and re-run.
