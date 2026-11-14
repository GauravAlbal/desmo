# Change: replace a reviewed configuration

The desk you are maintaining already implements the configuration rollout brief. Requirements R1 through R14 stay in force. This change revises none of them. It adds one command and requirements R15, R16, and R17.

The operator must be able to correct a configuration after it has been presented and, if they already approved it, after that approval, without starting a new state directory. An approval of the previous bytes must not authorize the corrected bytes. The corrected bytes become installable only through a new approval, and that path must succeed.

Replacement is refused once an install attempt has been saved or a terminal outcome has been committed. Use a new `STATE_DIR` for a later rollout. There is still no fleet transaction and no cross-device budget.

## Command

### `./run replace <proposal_id> <config_path>`

Allowed only when a proposal exists, `proposal_id` matches it, and `install` is `none` (no attempt record and no terminal outcome).

Capture the file bytes the same way `prepare` does: non-empty, at most 1 MiB, digest is lowercase hex SHA-256 of the raw bytes. Keep the same `proposal_id` and `device_id`. Set `approval` to `none` and `approved_digest` to `null`. Forget the previous approval; it must not remain as a hidden authorization. Changing the source file after `replace` returns must not change the captured bytes.

Success, exit 0:

```json
{"proposal_id":"...","device_id":"...","digest":"...","approval":"none","approved_digest":null,"install":"none"}
```

Refuse if the proposal id does not match, if the file is unusable, or if an attempt has been saved or `install` is `installed` or `rejected`. A refusal must not change the captured bytes, the approval, or the service. In particular, `replace` after a successful install must leave `installed_digest` and the service record as they were.

`show` after a successful replace presents the new bytes and the new digest, with approval cleared.

## Requirements

R15. Replace after review. After `prepare` (the configuration is available to `show`) and also after a successful `approve`, before `install` has saved an attempt, `replace` exits 0, keeps the proposal id and device id, presents the new file bytes and digest, and leaves `approval` `none` and `approved_digest` `null`. A later change to the replacement file does not change the presented bytes. After `install` has committed `installed`, `replace` is refused and does not change the installed digest or the service record.

R16. Old approval cannot authorize different bytes. After an approval and a successful `replace`, `approve` of the previous digest is refused and does not record an approval. `install` without a new approval of the new digest is refused, does not contact the service, and does not add an installation effect. The previous approval must not authorize the new bytes.

R17. Reapproval installs the corrected bytes. After that `replace`, `approve` of the new digest exits 0. A following `install` against a healthy service exits 0. `status` reports `installed` with `installed_digest` equal to the new digest. The bytes stored by the service equal the replacement file bytes as captured at `replace`, not the previously approved bytes.
