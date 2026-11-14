# @desmo/runtime

Typed Node adapter over the authoritative `ik` binary. TypeScript knows the
command shape; Desmo decides whether it may commit now.

This is another entrance to the same authority — not a second kernel, not a
TypeScript state-machine interpreter, and not a JavaScript reducer worker.

## Install

Install the runtime tarball and the platform tarball for your machine from the
GitHub Release (`npm install --ignore-scripts ./desmo-runtime-*.tgz ./desmo-<platform>-*.tgz`;
the exact commands are in the top-level `README.md`). The platform package
(`@desmo/darwin-arm64`, …) carries the qualified binary and the launcher errors if
it is absent. Nothing is fetched from a registry. Alternatively set `IK_BINARY` or put
`ik` on `PATH`.

Do not require Rust, Python, or a private credential for the demo.

Qualified deployment: Node/Python/Go **workers with persistent local storage**.
Not Vercel-style read-only filesystems, not multi-replica shared SQLite, not
the browser.

## Demo

```bash
npx --no-install desmo demo stale-worker
```

Reproduces a late worker after cancel, then the Desmo refusal, and writes a
local HTML/JSON trace. No cloud.

```bash
npx --no-install desmo init --template job --language typescript
npx --no-install desmo doctor --dir . --json
npx --no-install desmo skill install --dir .
```

`skill install` copies the packaged BUILD/CHANGE/DIAGNOSE router to
`.agents/skills/desmo/`. `doctor` reports whether that copy is present. It
does not overwrite a locally modified skill file unless `--force` is set.

`doctor` reports the installed package version against `ik --version`, project
checks, machines, scenarios, and adapter manifests under `.ik/adapters`. It
reads those files only. It does not execute an adapter or fetch a tool. A
missing optional tool is an `unavailable` line with the next command. Exit 1
only when the `ik` runtime cannot be found.

## Lossless integers

Protocol integers may exceed `Number.MAX_SAFE_INTEGER`. The SDK preserves them
as `bigint` and **rejects already-lossy `number` inputs**.

## Outcomes

| Result | Meaning |
|---|---|
| Committed | Kernel committed the transition. An external effect may still be pending. |
| Refused | Kernel rejected the command. Canonical state unchanged. |
| No-op | No domain transition committed. |
| Submission unknown | Request may have committed; reconcile before acting. |
| Effect outcome unknown | External execution may have happened; attributable outcome unresolved. |

Do not auto-refresh stale handles, blind-retry commands, or re-execute effects
after timeout.
