# Reserved-owner packet for the agent-DFY goal

This lists what the agent-owned work could not close and the smallest action that unlocks each item. Nothing here is waived, and no agent granted any of it. Evidence for each line is in `audits.md` and the batch records.

## Qualification state at the time of writing

- **Journey qualification floor**: met on first scoring in the second held-out lineage (`batch-011/`): build 6 of 6, change 6 of 6, continuity 6 of 6, diagnose 18 of 18, installed bytes from source `3741e2b`, macOS arm64, general-purpose agent with a shell, two replicas per family. The first lineage missed its floor (`batch-010/`) and stays on record.
- **DFYJourneyQualified**: not asserted. Open: the gate authority controls below, and the application and product findings in `audits.md` (disposition correction): business-rule enforcement and recovery correctness are not established for the affected rules.
- **EngineeringReady**: not asserted from batch counts.
- **PublicReleaseReady**: no.

## Items that need the owner or a human

| Item | Why it is not agent-closable | Smallest unlocking action |
|---|---|---|
| Gate authority: forged coverage and self-approval | Both were reproduced against an earlier installed gate on disposable targets and filed. The repair (receipt-bound coverage, approve separated from acceptance, a read-only ledger revalidation verb) is now in the gate's main history and in the installed gate (0.3.47 at the time of the re-run). Re-run of the same disposable-target controls: a forged minimal ledger line, a copied real line with a swapped field, and a real line for another revision are each `NOT COVERED` (exit 3), and approving an unaccepted revision is refused (`no_authentic_accepted_receipt`; an override is separately attributable). These four controls are closed at that scope and are not reopened under the different same-account threat model in the next row. | none for this row |
| Worker and acceptance-authority separation (no self-certification) | Today the installation is cooperative, same-account operation: the account that runs workers owns the acceptance event store and can run the operator-only verbs, so it can write a consistent event and ledger. Disposable-target demonstrations (read-only investigation, nothing on the real system changed): a deny-write sandbox or an immutable-file flag stops a cooperating child but not the owning account, which can clear the flag or relaunch unsandboxed; the operator verbs check an in-process boolean, not an operating-system identity. Mandatory signing is not recommended: a key readable by the worker account is not a boundary. The smallest boundary found uses existing facilities: a separate unprivileged operating-system user that owns the acceptance stores and runs the gate through a narrow non-interactive entry point whose argument list admits submission, status and coverage checks only, with operator verbs run by the operator under that user. Until it exists the no-self-certification property is not claimed. Revalidation row: 160 of 161 ledger rows bind; the remaining row cites an event file outside this repository that was not located or authenticated here and is left unverified, not recreated | authorize: a new unprivileged user and group; ownership change of the acceptance stores of the repositories that use the gate; installing the narrow entry point and one sudoers line for it (not for the gate CLI itself); retargeting worker submission through it. Optional: a hosting-side required coverage check. The agent can then run the prepared attack scripts against it |
| Gate authority: publication, pin, reroll controls | Executed locally with disposable targets (see `batch-011/audit/RC-controls.md` and `audits.md`): the repository's own release path has no authorization check (FAIL), this repository has no pin-update verb (FAIL), the gate's pin file and settled-rejection controls pass, and the CLI operator boundary fails for a same-account worker. Real registry permissions were not tested. | decide on an authorization step in the release path; provide a scoped sandbox registry if the remote-permission claim is wanted |
| Human review of the lifecycle surfaces | needs a human | assign a reviewer to the surfaces named in the readiness table |
| Human cold use | needs an unfamiliar human; the autonomy audit is not a substitute | run the study |
| Linux x64 | no x64 runner here; arm64 Linux source suite and packaged-artifact checks passed (`validation/linux/`) | provide an x64 runner or drop x64 from the advertised targets |
| Consumer acceptance | needs the consumer | consumer sign-off |
| Licensing, publication, public pins, owner decisions O-1 to O-7 | reserved to the owner | owner decisions |

## Product findings (open; not owner-only)

- Correction: the earlier claim that guards cannot add integers or assign string literals was wrong. At `3741e2b` guards support string literals, `not_exists` and integer plus or minus a constant; `then` assigns quoted string literals (`'"paid"'`), arguments and counter increments. Those forms were undiscoverable in the docs. Variable plus or minus variable, multiply by a constant and divide by a constant in a comparison were genuinely missing; a repair adds exactly those (see `batch-011/audit/RB2-language.md`). Frozen-byte results and the batch-011 score are unchanged.
- Open until reproduced evidence closes them: derived amounts, deadlines and spans copied from the host without checking their defining relationship (`batch-011/audit/RB1-authority.md`).
- A frozen machine definition cannot gain a command or rule and there is no migrator. `change.md` step 5b now says what to do; a migrator would be a product decision.
- No delivery in any batch used `lookup_operation`.
- A host gate that the command interface can bypass (N13 version-2 `checkin`) and leftover facts that make a later reconcile `corrupt_state` (N15) passed every instrument; they were found only by independent source inspection.

## What not to infer

Two replicas per family is a finite floor, not a success rate. Nothing here compares Ikaros with any other stack. Nothing here authorizes publication.
