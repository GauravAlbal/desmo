# Refund approval library

Build a Python 3.9 library in `app.py` (single module, in your working directory). It manages refund requests
that need approval before money is paid out. State must survive process restarts.

## API (exactly this)

```python
class Refused(Exception): ...        # a business rule forbids the action
class Stale(Refused): ...            # caller's expected_version is not the current version
class Conflict(Refused): ...         # same operation id or effect key reused with different content

class Refunds:
    def __init__(self, path: str): ...   # opens or creates durable state at `path`; a new instance on the same path sees all prior state
    def submit(self, refund_id, requester, amount_cents, operation_id=None) -> dict
    def edit(self, refund_id, amount_cents, expected_version=None, operation_id=None) -> dict
    def approve(self, refund_id, approver, amount_cents, expected_version=None, operation_id=None) -> dict
    def authorize_payout(self, refund_id, amount_cents, expected_version=None, operation_id=None) -> dict
    def record_payout(self, refund_id, outcome_digest, operation_id=None) -> dict
    def get(self, refund_id) -> dict      # {"state", "amount_cents", "version", "approver", "paid_digest"}
```

Every method returns the same dict shape as `get` after the action. `version` starts at 1 on submit and increases
by exactly 1 on each action that changes state.

## Rules

1. States: `draft`, `approved`, `payout_pending`, `paid`.
2. `submit` creates a `draft`. Re-submitting an existing id is refused.
3. An approver may not be the requester.
4. `approve` is only valid from `draft`, and `amount_cents` must equal the current amount (approval binds the exact amount).
5. `edit` is valid from `draft` or `approved`. Editing an `approved` refund discards the approval and returns it to `draft`. Editing in `payout_pending` or `paid` is refused.
6. `authorize_payout` is valid only from `approved` and only for the approved amount. It moves to `payout_pending`; at most one payout may ever be pending or completed per refund.
7. `record_payout` is valid only from `payout_pending`: it moves to `paid` and stores `outcome_digest`. Repeating it with the same digest is a no-op returning the current state (version unchanged). A different digest after `paid` raises `Conflict`.
8. `expected_version`, when given, must equal the current version or `Stale` is raised and nothing changes.
9. `operation_id`, when given, makes the call idempotent: repeating the same id with the same arguments returns the originally returned dict without any new change (even if the refund has since moved on); the same id with different arguments raises `Conflict`.
10. A refused action must change nothing (no partial state, no version bump).
11. Unknown refund ids raise `Refused`.

Deliver `app.py` and whatever else it needs next to it. You may write your own tests. A hidden acceptance suite
will import `app` from your directory and exercise the API above, including reopening the same path in a new
`Refunds` instance mid-flow.
