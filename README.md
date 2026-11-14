# Desmo

Hard lifecycle invariants for fast-moving Python, Go, and TypeScript systems.

> **Keep the code you change often in Python, Go, or TypeScript. Put the state
> transitions you cannot afford to get wrong in Desmo.**

Product: **Desmo**. Repository: `desmo`. Engine: `invariant-kernel`. Binary: `ik`.
Python: `invariant_kernel`. Go: `github.com/GauravAlbal/desmo/go`.
TypeScript: `@desmo/runtime`.

Desmo runs on the `invariant-kernel` engine (the `ik` binary, `.ik.toml` machine
files, `IK_*` variables). Schema identifiers such as `ikaros.trace.v2`, and the trace
`product` field, keep their original `ikaros` spelling so existing bundles stay valid.

The name: a desmodromic valve is driven positively in both directions rather than
relying on a spring. Desmo refuses any lifecycle transition unless a declared rule
and a current capability open it.

Alpha. Adopt from a **versioned GitHub Release**, not a
clone of this repo.

## Why

Orchestration and product code belong in Python, Go, or TypeScript. Race-sensitive
lifecycle authority — illegal transitions, stale capability, effect
identity, replay — belongs behind a Rust process boundary.

```text
host (Python / Go / TypeScript)
        ↓ typed client
ik   (canonical JSON)
        ↓
Desmo kernel  (authority)
```

The host never implements kernel lifecycle transitions. An external reducer
may implement deterministic domain reduction, but a copied handle that
mutates after a later command is refused by Rust (`IK_CAPABILITY_STALE`).

## 30-second example

```python
from invariant_kernel import Kernel

kernel = Kernel(store=".state/kernel.db")
job = kernel.machine("job.ik.toml").create("job-1")
job = job.start()
print(job.state, job.generation)
```

```go
k, _ := invariantkernel.Open(".state/kernel.db", invariantkernel.Options{})
m, _ := k.Machine(ctx, "job.ik.toml")
job, _ := m.Create(ctx, "job-1")
job, _ = job.Command(ctx, "start", nil)
```

## When to use it

- canonical lifecycle with illegal transitions
- retries / exhaustion
- approval states
- external effects with identity
- generation-bound authority
- replay-sensitive durable state

## When not to use it

- ordinary immutable data
- routing
- compilation / transformation
- business calculations
- workflow orchestration with no hard shared invariant

## Install

The commands below target the `0.5.0` release candidate artifacts. The release owner must
qualify the packed bytes and obtain source acceptance before publishing them;
source-suite success alone does not authorize publication or a `LATEST_RELEASE`
update. Consumers use the matching published manifest and checksums.

Download `ik-darwin-arm64.tar.gz` (Apple silicon) or `ik-linux-amd64.tar.gz` (x86_64 Linux; its
Node platform package is `linux-x64`) from the GitHub Release,
verify `SHA256SUMS`, put `ik` on `PATH` or set `IK_BINARY`.

```bash
gh release download v0.5.0 -R GauravAlbal/desmo \
  -p 'ik-darwin-arm64.tar.gz' -p 'invariant_kernel-0.5.0-py3-none-any.whl' -p SHA256SUMS
shasum -a 256 -c SHA256SUMS --ignore-missing
tar -xzf ik-darwin-arm64.tar.gz
install -m 0755 ik /usr/local/bin/ik
ik --version   # invariant-kernel 0.5.0 protocol 1.0: must equal your client's runtime version
```

Python (locates `ik` via `IK_BINARY` or `PATH`; the wheel does **not**
bundle the binary):

```bash
pip install --no-index --no-deps invariant_kernel-0.5.0-py3-none-any.whl
```

Node (no registry is involved; the runtime and the platform package are installed from the
release files, and the platform package carries the `ik` binary):

```bash
gh release download v0.5.0 -R GauravAlbal/desmo \
  -p 'desmo-runtime-0.5.0.tgz' -p 'desmo-darwin-arm64-0.5.0.tgz' -p SHA256SUMS
shasum -a 256 -c SHA256SUMS --ignore-missing
npm install --no-audit --no-fund --ignore-scripts \
  ./desmo-runtime-0.5.0.tgz ./desmo-darwin-arm64-0.5.0.tgz
npx --no-install desmo doctor --dir .
```

Use the platform package that matches your machine (`darwin-arm64`, `darwin-x64`,
`linux-arm64` or `linux-x64`). The runtime package does not list the platform packages as
dependencies, so installing from the two files contacts no registry for them.

Go:

```text
require github.com/GauravAlbal/desmo/go v0.0.0-<date>-<tag-sha>
```

Pin the release, not `main`. See [docs/upgrading.md](docs/upgrading.md).

## Define a machine

A `.ik.toml` file declares facts, states, commands, guards, effects,
and invariants. Commands are requested by name and args. Destination
states are not sent. `ik identity machine.ik.toml` prints the kernel
identity so clients never hash specs.

Start from `machines/verification_attempt.ik.toml`.

## Python external reducers

The Python package can bind a deterministic, versioned semantic reducer to a
machine:

```python
from invariant_kernel import Kernel

game = Kernel(store=".state/kernel.db").reducer(
    "my_game.reducer:WeddingWeekendReducer",
)
scene = game.create("wedding-weekend-01")
scene = scene.execute(
    {"name": "kiss_noah"},
    {"location": "dance_floor", "consent": True},
)
```

`Aggregate.execute(command: Mapping, observation: Optional[Mapping] = None)` is the
generic API; `command(name, **args)` remains unchanged. A reducer is loaded
from a stable `module:qualname` (no lambdas, closures, or callback transport)
and receives recursively immutable input trees. It owns deterministic
semantic reduction: deciding `noop` versus a versioned event and applying that
event to produce semantic state/effect/outcome.

Rust remains the sole authority for capability preflight, canonical
currentness, transaction/commit, journal, projection, replay, and
verification. Binding discovery invokes `describe` once; Rust recomputes the
reducer binding identity and initial-state digest, then invokes `decide` /
`apply` through strict protocol `1.0`. It commits nothing when the reducer or
worker fails. A stale capability is rejected before a command worker starts.
After binding, `create`, `record`, and `show` do not launch Python; semantic
launch is limited to `command`, `replay`, and `verify`.

The worker is trusted and cooperative, not an OS sandbox: Python receives no
store path, but the protocol cannot enforce OS-level IO denial. The
WEDDING-WEEKEND-01 / `D_KISS_NOAH` example above documents consumer shape
only; it does not claim that interactive fiction consumer is migrated. See
[docs/python.md](docs/python.md) for the descriptor, digest, worker, and
failure contracts.

There is no Go reducer API in this feature; the Go package remains a typed
transport client.

The `0.3.0` persistent-worker, session, and transaction contracts remain:
workers use newline-delimited JSON-RPC 2.0, are recycled after 5000 calls,
and are replaced after failure. `IK_REDUCER_PARANOID=1` checks calls against a
fresh worker (off by default). `ik --store <path> --session` and Python
`Kernel(session=True)` retain a machine/reducer and SQLite store across requests.
`Aggregate.transaction([...])` applies an ordered all-or-nothing command batch
under one capability; only the last step may create an effect intent.

## Region authoring (`0.4.1`)

Authoring schema `2` adds bounded compound and parallel regions, guarded
child-first parent fallback, and explicit atomic joint commands. It lowers to
the existing flat runtime definition; it adds no second execution authority.
Schema-1 sources remain supported. See [region lowering](docs/region-lowering.md)
for the contract, finite evidence domains, and packed-artifact qualification boundary.

## ik check

```bash
ik check machine.ik.toml
```

Developer analysis. Not a lifecycle operation. Exit 0 accept, 1 refuse.
`--json` `judgment.verdict` is `accept` or `refused`; `judgment.status`
is `accepted`, `missing_evidence`, or `witnessed_violation`.

## Brownfield

Move **one** command family first. Delete the host branch when Desmo
owns it. Do not keep both. Details:
[docs/brownfield-migration.md](docs/brownfield-migration.md).

## How authority is split

| Owner | Owns |
|---|---|
| Host | IO, scheduling, adapters, product orchestration |
| Desmo | destinations, guards, capabilities, effects, replay |
| Language client | envelopes, process, typed errors — **no** semantics |

Desmo does not learn product law. Hosts do not regain lifecycle
authority.

## Release / compatibility

Protocol `1.0`. Runtime `0.5.0`. Persisted store schema `4`
(`0.4.x` stores are schema `3` and migrate forward in place on first open; older
binaries refuse schema `4`, so back up first; see [docs/upgrading.md](docs/upgrading.md)).
Stores keep compact journal facts plus state digests; historical store schema `2`
fails closed. Authoring schema `2` is unrelated to store schema numbering.
`ik`, the Python client/worker, and the Go client expectation must match the
runtime version exactly, even when protocol and store versions are unchanged.
Consumers pin `release-manifest.json` (manifest schema `3`): version, source SHA,
artifact file, SHA-256.

Upgrade: published new tag → parity against **that artifact** → move the pin.
Rollback is the previous pin. Existing consumer pins are unchanged; this release does not migrate historical pins.

## Deeper docs

- [Greenfield](docs/greenfield.md)
- [Brownfield](docs/brownfield-migration.md)
- [Python](docs/python.md)
- [TypeScript](docs/typescript.md)
- [Go](docs/go.md)
- [Job retrofit example](docs/examples/retrofit-job/README.md)
- [Agent guide](docs/agent.md)
- [Coding-agent skill](skills/desmo/SKILL.md)
- [Machine evolution](docs/machine-evolution.md)
- [Publish-approval example](examples/publish-approval/README.md)
- [`ik check`](docs/ik-check.md)
- [Feasibility validation](docs/feasibility-validation.md)
- [Change-aware evidence](docs/change-evidence.md)
- [Scenarios](docs/scenarios.md)
- [Workbench report](docs/workbench-report.md)
- [Upgrading / pins](docs/upgrading.md)
- [Release compatibility](docs/release-compatibility.md)
- [Diagnostics](docs/diagnostics.md)
- [Public release checklist](docs/public-release-checklist.md)
