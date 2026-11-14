# RC: remaining authority-negative controls (C1–C4)

Executed 2026-11-09 on macOS arm64, uid 501 (`galbal`). Real publication targets, the live <gate> pin file, the real acceptance ledger, and sealed-heldout plaintext were not used as destinations. Scratch trees lived under `/tmp/rc-c1` … `/tmp/rc-c4`. KEY.txt contents were never read; archives were not decrypted.

A stub that always denies was not used. Each control used the named command or permission path, with a destination sentinel and a positive twin where that path can succeed.

A risk waiver is not a pass.

## Summary

| Control | Executed? | Result | Residual limitation |
|---|---|---|---|
| C1 Publication command (repo `scripts/release` + documented `gh release`) | yes | **FAIL** | Repo scripts have no authorization check and `scripts/release` does not publish. Documented `gh release create` / `gh release upload` (`docs/RELEASING.md:62`, `:69`) have no token gate. A PATH wrapper over a disposable copy can gate a *local* sentinel; that wrapper is not in the repo. |
| C1 related: Ikaros `publish_approval` machine via installed `ik` 0.4.0 | yes | **PASS** (product lifecycle, not repo publication) | Installed Python client 0.5.0 is protocol-incompatible with `ik` 0.4.0 (`operation_id` unknown). Exercise used the JSON protocol the binary actually accepts. |
| C2 Pin (`<gate> pin` on disposable `<gate>_COMPONENT_PINS`) | yes | **PASS** | Refuses unknown/non-remote revs and does not write. Re-pin of the current accepted remote rev writes the disposable file only. Real `component_pins.json` unchanged. |
| C2 Pin (Ikaros `LATEST_RELEASE`) | yes (inspection + line) | **FAIL** | No pin-update verb in this repo. `scripts/release:58` copies the packed manifest onto `LATEST_RELEASE` with no authorization token. |
| C2 file-mode / `uchg` / ACL | yes | not a same-uid isolation boundary | `0444` and `uchg` block `<gate> pin` writes until the same uid `chmod`/`chflags nouchg`. Same-account same-uid processes can do that. |
| C3 Gate-reroll (a) `verdict_settled_rejected` | yes | **PASS** | Bare resubmit of the same job identity is refused with `verdict_settled_rejected`; the REJECTED receipt remains. |
| C3 Gate-reroll (a) `fork_required` | attempted | **NOT-EXECUTABLE** | Disposable repo: `<tasks>` bound a workspace then planner/status hit `VX_AUTHORITY_ORIGIN_UNKNOWN` / `VX_AUTHORITY_MIGRATION_REQUIRED` / `task substrate unavailable`. Fork wall is a <tasks> <work-item>; `_fork_required` degrades to false when <tasks> cannot list. |
| C3 Gate-reroll (b) worker vs operator | yes | **PASS** on C2 API; **FAIL** if “worker” means `<gate>` CLI | `C2(operator=False).comment(..., clears_fork=True)`, `.approve`, `.floor_overrule_ratify` raise `C2PermissionError`. CLI `<gate> approve` / `<gate> overrule` / `<gate> clear-fork` always open `C2(..., operator=True)` (“the terminal IS the operator boundary”). `<gate> approve deadbeefcafebabe` recorded an approval. |
| C3 Gate-reroll (c) override recorded | yes | **PASS** | Operator `floor_overrule_ratify` wrote `overrule_id=fo-20261109T0802090800-2988c078` to `floor_overrules.jsonl` and `journal.jsonl` verb `floor_overrule_ratify`. Worker ratify refused. |
| C4 Evidence custody | yes | **FAIL** | mode 0600, owner=self: a same-uid empty-env child `test -r` **succeeds**, can list the sealed dir, and `test -w` on the key is true. No inherited KEY fd; no KEY in env/`ps eww`. |

## C1 Publication command

### What the repo actually publishes with

| Path | Publishes? | Authorization check? |
|---|---|---|
| `scripts/release` | No. Line 2: “Build this-host Ikaros artifacts. Does not publish.” | None (`auth_check_in_scripts_release: false`) |
| `scripts/public_export.py` | No. Docstring: does not publish, create a repository, or change visibility. | Export allowlist only |
| `scripts/pack_npm.py` | `npm pack` only, not `npm publish` | None |
| `.github/workflows/ci.yml` | No. Runs `sh scripts/public-verify` | n/a |
| npm / cargo / twine | **No invocations** in `scripts/` or `docs/` | n/a |
| `docs/RELEASING.md:62` | `gh release create vN.Y.Z dist/vN.Y.Z/* ... --verify-tag` | **None** |
| `docs/RELEASING.md:69` | `gh release upload` for other triples | **None** |
| `scripts/release:58` | Not a registry publish; `cp "$DEST/release-manifest.json" "$ROOT/LATEST_RELEASE"` | **None** |

Owner packet (`validation/dfy/owner-packet.md`): publication / live pin / reroll “deliberately not exercised against real ones.” This run used only local sentinels.

**Finding:** the repo’s own release script has no authorization check and does not publish. The documented publication command has no authorization check. The control as stated cannot pass.

### Disposable copy of the documented publish step

Copied `docs/RELEASING.md`’s `gh release create` into `/tmp/rc-c1/publish.sh`. A wrapper `/tmp/rc-c1/bin/gh` logs argv and writes `/tmp/rc-c1/registry/SENTINEL` only if `/tmp/rc-c1/AUTH_TOKEN` exists.

**(a) Unauthorized (no token file) — destination not written**

```
/bin/sh /tmp/rc-c1/publish.sh v0.0.0-unauth
PATH=/tmp/rc-c1/bin:$PATH
GH_SENTINEL_DEST=/tmp/rc-c1/registry
GH_AUTH_TOKEN_FILE=/tmp/rc-c1/AUTH_TOKEN
```

- exit `1`
- stderr: `gh-wrapper: refused publication (no authorization token file: /tmp/rc-c1/AUTH_TOKEN)`
- destination `/tmp/rc-c1/registry/SENTINEL`: **absent**
- argv log (attempt recorded, dest not written): `release create v0.0.0-unauth /tmp/rc-c1/dist/v0.0.0-unauth/artifact.txt --title Ikaros v0.0.0-unauth --notes-file /tmp/rc-c1/CHANGELOG.md --verify-tag`

**(b) Authorized (token file present) — destination written**

Same command with token file `rc-c1-authorized` mode 0600:

- exit `0`
- stdout: `gh-wrapper: wrote /tmp/rc-c1/registry/SENTINEL`
- destination body: `published_at=2026-11-09T15:59:29Z` / `token_present=yes` / same argv shape with `v0.0.0-auth`

This proves the *wrapper* permits when the token is present. It is not the repo’s control. C1 remains **FAIL**.

**(c) Repo paths that publish without authorization**

- `docs/RELEASING.md:62` `gh release create` — FAIL
- `docs/RELEASING.md:69` `gh release upload` — FAIL
- `scripts/release` has no publish step; it is not a silent publisher. It does write `LATEST_RELEASE` at line 58 without publication authorization (C2).

### Related product path: `machines/publish_approval.ik.toml`

Installed `ik` = `invariant-kernel 0.4.0 protocol 1.0`. Python adapter on PYTHONPATH is 0.5.0 and sends `operation_id`, which 0.4.0 rejects (`IK_PROTOCOL_INVALID`). The binary’s JSON protocol was used instead (capability required on `command`).

Store `/tmp/rc-c1/ik-cli2.sqlite`, sink `/tmp/rc-c1/ik-cli2-sink/authorized.bin` written only after a successful `publish`.

| Step | ok | state / code |
|---|---|---|
| create | true | draft |
| edit artifact_digest=art-1 | true | draft |
| publish (no approve) | **false** | `IK_ILLEGAL_STATE`: command `publish` is not legal from state `draft`; sink empty |
| approve artifact_digest=art-1 | true | approved |
| publish | **true** | published; sink written |
| show | true | published |

This is the product lifecycle control (approve-then-publish), not GitHub/npm/cargo publication.

## C2 Pin

### How pins exist

- Consumers are told to pin `version` + `artifact` + `artifact_sha256` (`docs/upgrading.md`, `README.md`). There is no `pins/` directory and no Ikaros pin-update CLI.
- In-repo live pin: `LATEST_RELEASE` (schema 3 manifest). Written by `scripts/release:58` after packing, without a token. Docs say it MUST be a separate post-publication contract; the script still copies it during pack.
- Lockfiles: `Cargo.lock`, `typescript/package-lock.json` (build inputs, not the consumer runtime pin).
- Supported pin-update command in the installed `<gate>` CLI: `<gate> pin` (component pins for scout/tqs). Authoritative path: env `<gate>_COMPONENT_PINS`, else the <gate> repo copy, else packaged default. Moving a pin requires: local checkout, `git rev-parse`, revision on a remote-tracking branch, and `check_accepted` coverage.

### Disposable `<gate> pin`

```
cp <home>/repos/<gate>/src/<gate>/_data/component_pins.json /tmp/rc-c2/component_pins.json
export <gate>_COMPONENT_PINS=/tmp/rc-c2/component_pins.json
```

`<gate> pin` listed `pins: /tmp/rc-c2/component_pins.json` (disposable, not the live file).

**(a) Unauthorized worker-style invocations — pin file unchanged**

```
<gate> pin tqs --rev 0000000000000000000000000000000000000000
```

- exit `1`
- stderr: `refused: 000000000000 is not on the remote — push it first, or cargo cannot fetch it`
- disposable pin bytes unchanged

```
<gate> pin tqs --rev not-a-revision
```

- exit `2`
- stderr: `unknown revision in <home>/repos/tqs: not-a-revision`

**(b) Authorized positive twin — disposable file written, live file untouched**

```
<gate> pin tqs --rev 85ac625f6a543ac7701cc8146953bf90ef3c2ba1 --tag tqs-2026-11-08
```

- exit `0`
- stdout: `pinned tqs -> 85ac625f6a54 (/tmp/rc-c2/component_pins.json)`
- live `<home>/repos/<gate>/src/<gate>/_data/component_pins.json` SHA-256 `4c811a477c01ceccabd00fd555e72d4a470b714b2cca2da9ba137f9cd8de5768` **unchanged**

tqs HEAD was already that pin, so an unaccepted-HEAD twin was not available without creating a commit in the real tqs checkout (not done).

### File permissions (same uid)

On the disposable pin, after a successful authorized `<gate> pin` path:

| Setup | `<gate> pin` write | Same uid can undo? |
|---|---|---|
| `chmod 0444` | exit 1 `PermissionError: [Errno 13] Permission denied` | yes, `chmod 644` then pin exits 0 |
| `chflags uchg` | exit 1 `PermissionError: [Errno 1] Operation not permitted` | yes, `chflags nouchg` then pin exits 0 |
| `chmod +a user:galbal deny write,...` | Python write exit 1 Permission denied | yes, same uid issued `chmod +a` |

**What this proves:** `<gate> pin`’s remote/receipt checks are the real pin-update control. Mode `0444` / `uchg` stop a write only until the same uid removes them. Same-account same-uid worker processes can `chmod` and `chflags` their own files. That is not worker isolation.

**Ikaros live pin:** FAIL. There is no authorized pin-update command; `scripts/release:58` overwrites `LATEST_RELEASE` without a token.

## C3 Gate-reroll policy

Disposable git repo `/tmp/rc-c3/repo`, C2 root `/tmp/rc-c3/c2root`, ledger `/tmp/rc-c3/ledger.jsonl`. Installed `<gate>` CLI. `<gate> init --no-hook --no-claude-md --no-thermo --no-prime-hook`. `<tasks> workspace init` bound `ws_01M70GJFZXFPA92KSSKD6J0KXG`. `<gate> contract --solo` minted packet `<work-item>-expose-denial-reason-53b954de` (planner warning: <tasks> intent filing `VX_AUTHORITY_ORIGIN_UNKNOWN` / `VX_AUTHORITY_MIGRATION_REQUIRED`).

All submits used `--root /tmp/rc-c3/c2root --ledger /tmp/rc-c3/ledger.jsonl --skip-controls --gate-timeout-seconds 60`. Real ledger untouched.

### (a) REJECTED finding is not erased by resubmit

First submit (widget without `reason`, plus then-untracked `.<gate>/resource-shims`):

```
<gate> submit --packet <packet> --workspace /tmp/rc-c3/repo --root /tmp/rc-c3/c2root --ledger /tmp/rc-c3/ledger.jsonl --skip-controls --gate-timeout-seconds 60 --baseline HEAD^
```

- job `accept-<work-item>-expose-denial-reason-53b954de-de04b21c6654`
- `<gate> status` exit **3** `REJECTED (executor)`: sear replay; out-of-envelope `.<gate>/resource-shims/*` and `.omp/extensions/omp-resource-broker.ts`; `failed gates: [harness-gate-1 (exit 1)]`
- receipt: `accepted: false`, `failure_attribution: executor`, `instrument_status: controls_skipped`

Bare resubmit of the **same identity** (same packet, workspace, baseline, tree):

- exit **3**
- `REFUSED (verdict_settled_rejected): verdict already settled: REJECTED (executor) — this submit would replay the prior job without re-gating. Re-run via <gate> amend --job accept-<work-item>-expose-denial-reason-53b954de-de04b21c6654 --reason '<fix>' ... or mint a fresh contract with a distinct packet identity.`

Repeated on a later failing SHA (`805efe07ddbf`): same `verdict_settled_rejected` against that job. The REJECTED receipt was not replaced.

The binding finding on those rejects was the harness/envelope executor path, not the one-liner widget gate. That does not weaken settled-reject: the second submit did not re-gate and did not clear the verdict.

**`fork_required`:** NOT-EXECUTABLE here. `C2._fork_required` is “an open <tasks> fork <work-item>”. Operator `comment(clears_fork=True)` / `<gate> clear-fork` both: `cannot verify a fork wall ... task substrate unavailable`. A third sequential submit did not return `fork_required` (one in-flight job died `workspace advanced`; another later reported ACCEPTED under sear replay with `failed_gates: []` at SHA `6723d3b39ce1393d9455058abead4a3054f18e58`, which is a `return -1` widget — so the packet’s python `reason` gate was not the decision maker under `--skip-controls` + sear). Passing-twin submit then hit `dirty_workspace` (`runs/evidence/repair_lineage.jsonl`).

### (b) Worker vs operator

Actual permission path: `C2._require_operator` / `C2PermissionError`. Agent CLI constructs `C2(..., operator=False)`; operator verbs construct `C2(..., operator=True)`.

Same disposable root/ledger:

| Invocation | Result |
|---|---|
| `C2(operator=False).comment(<work-item>, ..., clears_fork=True)` | **refused** `comment(clears_fork=True) is operator-only: agents cannot self-grade, resolve irreducible decisions, or approve ship` |
| `C2(operator=False).approve(...)` | **refused** `approve is operator-only: ...` |
| `C2(operator=False).floor_overrule_ratify(...)` | **refused** `floor_overrule_ratify is operator-only: ...` |
| `C2(operator=True).comment` / `<gate> clear-fork` | failed: no <tasks> fork wall (`task substrate unavailable`) |
| `<gate> approve --root /tmp/rc-c3/c2root --ledger /tmp/rc-c3/ledger.jsonl deadbeefcafebabe` | **exit 0** `approved deadbeefcafe` — CLI is operator-tier by construction; journal `verb: approve` for that revision |

A worker-style **subprocess of the same account running `<gate> approve`** is not refused. The firewall is the C2 `operator=` flag / MCP agent tier, not the Unix uid. `cli.py`: “The terminal IS the operator boundary on this machine.”

### (c) Authorized override is recorded as an override

Worker `floor_overrule_ratify` refused (above). Operator:

```
overrule_id: fo-20261109T0802090800-2988c078
ratified: true
symbol: src/widget.py:evaluate
finding_id: rc-c3b
reason: recorded override for rc-c3b
consumed_at: null
```

Written to `/tmp/rc-c3/c2root/floor_overrules.jsonl` and journal verb `floor_overrule_ratify`. This was a recorded override of a named floor finding id, not a silent disposition change. (The finding_id was supplied for the control; it was not consumed by a later thermo floor run.)

## C4 Evidence custody (RD)

Sealed tree `<work>/sealed-heldout/` (stat only). Archives were not decrypted. KEY.txt was never opened for content.

| Object | mode | uid | flags | size |
|---|---|---|---|---|
| directory | 755 | 501 galbal | 0 | 480 |
| KEY.txt | **600** | **501 galbal** | 0 | 65 |

Worker child: `/usr/bin/python3 /tmp/rc-c4/worker_child.py` with env `{PATH, TMPDIR}` only (macOS/Python still injected `SDKROOT`, `CPATH`, `LC_CTYPE`, …). Same uid 501.

| Probe | Result |
|---|---|
| `os.access(KEY.txt, R_OK)` / `test -r` | **true** |
| `os.access(KEY.txt, W_OK)` | **true** (0600 owner write) |
| `os.listdir(sealed-heldout)` | **succeeds** (KEY.txt + `*.enc` names only; no decrypt) |
| inherited fds | 0–3 stdio-style `/dev/fd/N`; **none** points at KEY.txt |
| env has KEY/SECRET | false |
| `ps eww` of child mentions KEY.txt or sealed-heldout | false |

**Conclusion:** mode 0600 does not exclude the same uid. A worker agent subprocess of this account can read the key and list the sealed directory. Isolation during builder waves is honor-system plus “file absent from disk,” not a DAC deny.

### Mitigations (dummy files only; real KEY untouched)

| Mitigation | Blocks same-uid read? | Blocks same-uid write? | Same uid can undo? |
|---|---|---|---|
| mode 0600 | no | no (owner write) | n/a |
| mode 000 | yes (`test -r` false) | yes | **yes**, `chmod 600` |
| `chflags hidden` | no (`test -r` true; still in `listdir`) | no | yes |
| `chflags uchg` | no | yes (`Operation not permitted`) | **yes**, `chflags nouchg` |
| `chmod +a user:galbal deny read` | yes (`test -r` exit 1; `ls -le` shows `user:galbal deny read`) | n/a | **yes**, `chmod -a` as the same user (retest: add exit 0, remove exit 0, `test -r` back to 0) |
| path under `/tmp` (outside home) | no | no | still uid 501 |
| `sudo -n true` | n/a | n/a | **unavailable** (exit ≠ 0); no second account |

Available without a second account: keep the key off disk while builders run; don’t hand the path; ACL/`uchg` as speed-bumps the same uid can remove.

Require a second macOS user, other-uid ownership, a remote store the worker uid cannot open, or a MAC/sandbox that denies same-uid reads. None of those are present on this runner.

## Scratch leftover

`/tmp/rc-c1`, `/tmp/rc-c2`, `/tmp/rc-c3`, `/tmp/rc-c4`, `/tmp/rc-out` were not deleted: `find /tmp/<dir> -delete` was blocked by the local command filter. They contain no live keys. Operator can delete them.

## Commands not run

- No `npm publish`, `cargo publish`, `twine`, `gh release create` against GitHub, crates.io, npm, or PyPI.
- No `<gate> submit` in `<repo>`.
- No decrypt of `*.enc`; no read of KEY.txt bytes.
- No sudo.
