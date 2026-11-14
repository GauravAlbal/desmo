# IK-06 author review, then independent review

Performed by the author of the change. The roadmap requires an independent reviewer for false
greens, bypasses inside the cooperative boundary, bad uncertainty semantics and integration
omissions. **No independent reviewer was available; that task is open.**

Findings from the author's own adversarial pass, all closed in this change:

1. **A mutant that never fired.** The first `SkipOperationBinding` seam sat in `bind_operation`,
   which the `command` and `record` paths do not use (they call `insert_operation` from
   `finish_step`). The kill test would have "passed" against a seam that never ran. Closing it:
   `with_law_mutant` now panics when its seam is not reached; the seam moved to `insert_operation`,
   the single row-write choke point. All 11 pre-existing mutants also fire under the strict rule.
2. **Name-only ledger.** `every_constitutional_mutant_is_named_in_the_killing_ledger` only checked
   that names were non-empty. `qualify.py` now requires each named test to exist, not be ignored,
   and to execute and pass; the exhaustive match keeps the enum and ledger in step.
3. **Unrun IK-03 controls.** Previously only one operation-attribution mutant had been run. The
   duplicate-gate and capability-in-fingerprint mutants now exist and are killed, and genesis
   validation (`SkipGenesisValidation`) has its own control.

Open (not closed here):

- Independent review (above).
- Client transport claims have no deleted-wall control.
- The expected relations in `region_contract.rs` are existing assets; this change did not
  re-derive them independently.
- macOS arm64 only.

## Independent review (separate agent) and closure

A separate agent reviewed the manifest and seams. Production walls (operation row, currentness, genesis) were not
found bypassed. Findings and closure:

| Finding | Closure |
|---|---|
| Duplicate-gate and fingerprint mutants covered the command path only | seams moved to `resolve_duplicate` / `fingerprinted_capability`; mutants renamed `SkipDuplicateGate`, `OmitCapabilityFromFingerprint`; kills added for transaction, effect record, create-with-command |
| Node rows could run a stale `typescript/dist` | `qualify.py --run` compiles TypeScript first and fails if it errors |
| Python dead session relabeled an in-flight operation as unsent | `Session` keeps in-flight ids submission-unknown; test added |
| Go `Command`/`Record` carried no operation id | both bind a generated id; test added |
| Replay evidence rows did not fold a journal | replaced with `ik_prop_001`, `replay_reconstructs_projection_and_verify_passes`, `altered_domain_state_digest_refuses_replay` |
| Genesis kill covered `create_aggregate` only | `skipping_genesis_validation_is_killed_for_create_with_command` |

Not examined by the reviewer: a fresh `--run`, Linux, dist-vs-source diff, Hegel, hostile or multi-replica use.
