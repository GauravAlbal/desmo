Toolchain: ordinary stack. Use Node/TypeScript or plain JavaScript with any reasonable local libraries or built-ins.

Competence baseline (the comparison is only fair if this arm is a competent transactional implementation, so these
are REQUIRED, not optional): keep the lifecycle's canonical facts in one persistent store (for example `node:sqlite`)
and commit every state change, its history record and any external-effect intent in ONE transaction; keep a durable
outbox / effect-intent record written in that same transaction, so a crash can never leave a committed transition with
a lost effect intent or an effect intent without its transition; apply a per-aggregate currentness check (version
or compare-and-set) so a request based on stale state is refused without mutating anything; make repeated submissions
of the same operation idempotent through a durable operation record; keep an append-only history from which the state
can be re-derived and checked; recover correctly after a restart at every crash boundary the brief names. Refusals
must not mutate authoritative state. Do not weaken any requirement to make this easier.
