# Account notification preferences

Build a local notification desk, one process per command, using durable `STATE_DIR` and a supplied `SERVICE_URL`. Entry point: executable `./run` in the project directory. The harness starts `stub/server.mjs <ready-file>` and sets `SERVICE_URL` to its public base URL; see `stub/CONTRACT.md`. Local concurrent processes and restarts are in scope; no distributed transaction is assumed.

Every invocation MUST write exactly one JSON object to stdout and no other stdout text. Exit 0 means the command completed with a result, including `suppressed` and `uncertain`; a refusal or command failure exits nonzero and writes `{"error": <reason string>}`. Successful preference responses contain `account_id`, `enabled`, `channel`, and positive integer `version`. `status` MAY also include the latest recorded delivery `phase`; that optional field is not required to report preference state.

Commands are `subscribe <account_id> <channel>`, `unsubscribe <account_id>`, `status <account_id>`, and `deliver <account_id> <message_id> <body>`. Account and message ids match `^[A-Za-z0-9_-]{1,64}$`; channel is `email` or `sms`; body is 1..256 Unicode code points.

`subscribe` refuses an invalid account id or channel without changing state. For an unknown account it creates an enabled preference at version 1 with the requested channel. For every existing account, `subscribe` succeeds, enables it, sets the requested channel, and increments its current version exactly once, including when it was already enabled or the requested channel is unchanged. Re-subscribing after `unsubscribe` is therefore permitted; it re-enables the account and increments the disabled version. An existing quiet flag is preserved.

`unsubscribe` refuses an invalid id or unknown account. On an enabled account it disables the preference and increments the version exactly once. On an already-disabled account it succeeds idempotently without incrementing the version. It preserves the selected channel and quiet flag. `status` refuses an invalid id or unknown account, reads local state only, and makes no service call.

A delivery id is fixed as `<account_id>:<message_id>:v<version>` and is sent as the idempotency key. The first delivery attempt binds the message id and body to that account version; the same body reuses that versioned delivery, while a different body for the same message id/version refuses conflict without creating a provider effect. A different message id is independent. A successful delivery result contains `message_id`, `delivery_id`, positive integer `version`, and `phase`; `provider_id` is present only for `delivered`. The phase values are `queued`, `uncertain`, `delivered`, and `suppressed`.

A delivery requested while unsubscribed is suppressed locally and never sent. For R4, the provider may commit a delivery and drop its response: the first command returns exit 0 with `phase: "uncertain"`. Recovery MUST GET the exact delivery id before any retry POST. If GET finds the committed result, recovery returns `phase: "delivered"` with the matching provider id, performs no further POST, and leaves exactly one provider effect. If the process stops after sending POST but before recording its response, the next process MUST recover using the same lookup-first rule.

For R5, start a delivery at version 1 and hold its provider response after the provider commits the POST. `unsubscribe` changes the preference to disabled version 2; `subscribe <account_id> sms` then succeeds with enabled version 3. Releasing the old response MUST return `phase: "delivered"` for the original version-1 delivery, without rewriting current preference. Status remains enabled on `sms` at version 3, and a new message uses a version-3 delivery id.

R1. A fresh subscribe persists enabled preference/version 1 without a service call; status in a new process sees it. Invalid identifiers/channel and delivery for an unknown account produce refusals without service calls.
R2. Unsubscribe persists disabled state and increments version; repeated unsubscribe while disabled is idempotent, succeeds, and does not increment again.
R3. Deliver while disabled returns suppressed and makes no service call. Deliver while enabled uses the exact versioned delivery id and returns delivered/provider id on healthy service; repeat does not duplicate effect.
R4. A dropped reply returns `uncertain`, not `suppressed`. Recovery looks up the same versioned id first; when it finds the committed delivery it returns `delivered`, makes no retry POST, reconciles exactly one effect, and never claims not-sent. After a process stop following POST, a restarted invocation uses the same recovery rule.
R5. Preference changes during in-flight delivery do not rewrite its captured version. A late response updates that delivery only, and cannot rewrite current preference or later delivery identity.
R6. Conflicting body for a previously bound message id/version refuses conflict and makes no new provider effect. A new message id is legitimate and must be deliverable.
R7. Local status is durable and does not call the provider. Unknown command and missing `SERVICE_URL` produce refusals.

