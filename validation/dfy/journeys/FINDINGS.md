> Status note: this file records the findings against the packed 0.5.0 binary of batch 017. The check verdict vocabulary is now `accept` / `refused` with a typed `status` (`accepted`, `missing_evidence`, `witnessed_violation`), `ik --help` lists every dispatched subcommand, and `ik explain` is documented as not a command. The remaining sections below are kept as the historical record.

# Findings (journey stepper vs packed `ik` 0.5.0)

Stepper: 18/18 steps passed against the packed binary. These are product
readout issues the model did not paper over.

## Check JSON verdict tokens are not one word

`ik check --json` writes `judgment.verdict` as:

| Case | `judgment.verdict` | exit |
|---|---|---|
| corpus + profile, obligations witnessed | `accept` | 0 |
| no corpus (missing evidence) | `refuse` | 1 |
| invariant-refusal witness (`fixtures/probe.ik.toml`) | `refused` | 1 |

Docs name ACCEPT / REFUSED. The JSON lane uses three tokens. A reader
who greps for `refused` misses the missing-corpus refusal.

Commands:

- `ik check fixtures/solo/tagged_item.ik.toml --json` → `"verdict":"refuse"`
- `ik check fixtures/probe.ik.toml --json` → `"verdict":"refused"` plus
  `invariant_witnesses[0].diagnostic.code=IK_INVARIANT_FAILED` `rule=x_limit`

## `ik explain` is not a packed CLI command

`ik explain` is not dispatched. The binary falls through to the store
USAGE and requires `--store`. Explanation exists as a library seam only.
The journey model uses `ik check --json` (witness + diagnostic) and the
runtime envelope `diagnostic.code` / `diagnostic.rule` instead.

## `--help` omits working subcommands

`ik --help` lists store ops, `check`, `identity`, `laws`, `schema`,
`profile diff`, `evidence verify`, `graph export`. It does not list
`scenario`, `report`, `handoff`, `corpus`, `analysis`, or `regression`,
which the same binary runs.

## Guard failures share `IK_INVARIANT_FAILED`

A `text_in` grammar miss (empty `id` on `tag`) returns

```json
{"ok":false,"diagnostic":{"code":"IK_INVARIANT_FAILED","rule":"tag.when"}}
```

not a distinct grammar-attack code. The rule id is `tag.when`. State is
unchanged (same capability still current). Docs table-row
`IK_INVARIANT_FAILED` as "Domain invariant"; guards use the same code.

## No kernel `legacy_request` diagnostic

A v2 command presented against a v1 aggregate returns
`IK_MACHINE_MISMATCH` (no `rule` field). Sending the new command name
through the v1 definition returns `IK_COMMAND_INVALID` `rule=command.unknown`.
`legacy_request` is a host mapping, not a packed-kernel diagnostic.

## Lookup of a committed op requires the original capability

`lookup_operation` with the current (post-commit) capability returns
`outcome=conflict`. The same body with the capability that accompanied
the original attempt returns `committed_exact`. Easy to misread as
"the operation conflicted" rather than "this lookup body does not match
the recorded fingerprint."

## `ik handoff` will not mark a lookup as `exercised`

Cited evidence id `$TMP/out/sc-lost#item-lost/found` resolves. The
finding status is `committed_exact`. Handoff claim status is
`incomplete`, because supporting statuses are `committed`, `refused`,
and `holds_in_scope` only. A lost-reply recovery that did the right
read cannot be marked exercised from that finding.

## Report artifact paths are absolute

`ik report --json` embeds absolute run-directory paths in
`findings[].artifacts` and finding ids. Harmless locally; noisy if the
document is copied.

## Not defects

- `ik check --plan` exit 0 with schema `ik.check.plan.v1` and no witness:
  matches the documented "does not explore" shape.
- Duplicate of a committed operation: `ok=true`, `historical=true`,
  `mutation=false` (envelope) / scenario step `outcome=historical`.
- `verify` after operate: `verified=true`, `journal_ok=true`,
  `projection_ok=true`.
- Old instance after v2 authoring: v1 `recall` still commits; v2
  `extend` on that aggregate is `IK_MACHINE_MISMATCH`.
