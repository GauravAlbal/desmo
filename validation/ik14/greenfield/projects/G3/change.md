# Follow-on: pause and resume

You are maintaining the import desk from `brief.md`. Keep every original requirement. This change revises none of R1–R14. It does not relax the retry limit, the single-effect rule, attribution of uncertain sends, or the rule that a late reply must not overwrite a newer delivered outcome.

It adds a local pause. Pause is not cancellation. The desk still has no way to undo, retract, or reverse a request already sent to the destination. The dispatch boundary in the brief stays in force: once the request has been sent, the attempt is `in_flight` until lookup or an applied matching success confirms delivery, or until a confirmed rejection is recorded under the original rules. Pause only stops a new send.

## New commands

```text
./run pause <import_id>
./run resume <import_id>
```

They print the same status object as the other commands. `phase` may now be `paused`. The other phase and dispatch values are unchanged. `pause` and `resume` must not contact the destination. They must not change `attempts_failed`, `destination_id`, or `last_error`.

`pause`:

- Unknown id: `{"error":"not_found"}`.
- `delivered` or `failed`: `{"error":"not_pausable"}`, state unchanged, no destination call.
- `ready` / `none`: phase becomes `paused`, dispatch stays `none`.
- `uncertain` / `in_flight`: phase becomes `paused`, dispatch stays `in_flight`. Do not report this as not sent. Do not clear the in-flight mark.
- Already `paused`: return the current status unchanged (idempotent).

`resume`:

- Unknown id: `{"error":"not_found"}`.
- Not paused: `{"error":"not_paused"}`, state unchanged.
- `paused` / `none`: phase becomes `ready`, dispatch stays `none`.
- `paused` / `in_flight`: phase becomes `uncertain`, dispatch stays `in_flight`. This is not a new send and does not contact the destination.

`advance` while `paused` / `none` returns `paused` and must not contact the destination.

`advance` while `paused` / `in_flight` must not POST. It may lookup. If lookup shows the same record, commit `delivered` / `confirmed` with that destination id and null `last_error` (pause cannot hide a delivery and cannot undo it). If lookup shows a different record, the original conflict rule applies. If lookup misses or is unavailable, stay `paused` / `in_flight` and do not POST.

## Requirements added

**R15. Pause blocks a new dispatch and resume continues.** `pause` on a ready import returns `paused` / `none` with the same attempt count. A second `pause` is the same status. `advance` while paused makes no destination call. `resume` of an unknown id is `not_found`. `resume` of an import that is not paused is `not_paused`. `resume` of a paused, not-in-flight import returns `ready` / `none` with the same attempt count, and a later `advance` can deliver it.

**R16. Pause does not undo in-flight work.** After `CRASH_AT=after-dispatch-before-result`, `pause` returns `paused` / `in_flight`, does not contact the destination, and does not report the attempt as not sent. `resume` returns `uncertain` / `in_flight` with the same attempt count and still does not send. A following `pause` and then `advance` reconciles by lookup to `delivered` / `confirmed`, with the lookup destination id, exactly one effect, and no second POST.

**R17. Pause does not restore retries.** An import with `attempts_failed` 2 can be paused and resumed and still has 2. The next confirmed rejection still exhausts the limit at 3. `pause` of that `failed` import is `not_pausable` and leaves `failed` / 3 in place, including for a new process. `advance` after that refusal does not contact the destination.

No original requirement is removed or weakened. In particular, R5 and R8 still hold for paused imports: exhausted retries stay exhausted, and an in-flight send whose reply was lost is still attributed by lookup rather than reported as not sent or repeated under a new key.
