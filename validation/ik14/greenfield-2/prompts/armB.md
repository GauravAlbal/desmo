Toolchain: you have been assigned the Desmo runtime for the critical lifecycle. It is provided as local npm tarballs
under `KIT/npm/` (install both into your project, for example `npm install --offline --no-audit --no-fund
KIT/npm/desmo-runtime-0.5.0.tgz KIT/npm/desmo-darwin-arm64-0.5.0.tgz`; if npm cannot run, extract the tarballs
manually) with its guide at `KIT/README.md`, `KIT/docs/` and `KIT/examples/`. Use Desmo for the critical lifecycle
rules and their persistence; use ordinary libraries or built-ins for the rest of the application. Do not modify the
installed runtime. The machine definitions and your application code are yours to write.

The same competence baseline as the other assignment applies to your result: the lifecycle's canonical facts, every
state change with its history record and any external-effect intent must commit atomically; a crash must never leave
a committed transition with a lost effect intent or the reverse; stale requests must be refused without mutation;
repeated submissions of the same operation must be idempotent; the state must be re-derivable from history; recovery
after restart must be correct at every crash boundary the brief names; refusals must not mutate authoritative state.
Do not weaken any requirement to make this easier.
