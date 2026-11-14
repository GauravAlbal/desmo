# Change request

Change the existing refund library (`app.py` in your working directory; the original spec is /tmp/ik-blind/SPEC.md) as follows:

**Two distinct approvers are required.** `approve` records one approval; a refund moves from `draft` to `approved` only once two *different* approvers have each approved the current amount. Until then `approve` returns the refund still in `draft` (version still increases by 1 per recorded approval). The same approver approving twice is refused. The requester still cannot approve. Editing an `approved` refund, or editing a `draft` that has one approval, discards all recorded approvals. `get` must report `approver` as the approver whose approval completed the pair (or None while not approved). All other rules and the API are unchanged; existing persisted state from the old behavior need not be migrated.

Keep the existing tests meaningful (update the ones the change legitimately alters). Reply with what you changed, and which rules you are unsure of.
