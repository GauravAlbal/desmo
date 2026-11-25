# TypeScript

Package: `@desmo/runtime`. Thin Node client over `ik`. Runtime remains
authoritative.

```ts
import { Kernel } from "@desmo/runtime";

const kernel = new Kernel(".state/kernel.db");
const jobs = await kernel.machine("job.ik.toml");
const job = await jobs.create("example");
const admitted = await jobs.createWith({ aggregateId: "admitted", command: { name: "start" }, operationId: "op-1" });
const running = await job.command("start");
await running.command("cancel");
await running.command("submit", { artifact_digest: "a", baseline_digest: "b" });
// StaleCapabilityError: this handle is stale.
```

See `typescript/README.md`. Trace export is advisory HTML/JSON
(`ikaros.trace.v2`); it is not authorization.

Install from the release files, not a registry (the commands are in the top-level `README.md`):
the runtime tarball and the platform tarball for your machine (`darwin-arm64` or `linux-x64`), installed together with
`npm install --ignore-scripts`. The commands below then run from the installed package, so use
`npx --no-install` to stop npx fetching anything:

- `npx --no-install desmo demo orders`
- `npx --no-install desmo demo stale-worker`
- `npx --no-install desmo simulate <machine.ik.toml> [--out file.html]`
- `npx --no-install desmo init --template job --language typescript`
- `npx --no-install desmo doctor [--dir project] [--json]`
- `npx --no-install desmo skill install [--dir project] [--target agent-skills-dir] [--force]`

The runtime package does not list the platform packages as dependencies, so installing the two
tarballs contacts no registry for them. Install only the tarballs from the release, verify
`SHA256SUMS`, and check that `node_modules/@desmo/` contains only the runtime and your platform
package. The `@desmo` npm scope is not registered by the owner.

`doctor` compares `@desmo/runtime` with `ik --version` and reads
`.ik/checks.toml`, `*.ik.toml`, `scenarios/`, and `.ik/adapters/*.json`. It
does not execute an adapter or fetch anything. Missing optional tools are
`unavailable` lines, not a failure of native check or scenario commands.
`doctor` also reports whether `.agents/skills/desmo/` is present.

`desmo skill install` copies the packaged skill into that directory. It is
idempotent. A locally modified file is left unchanged unless `--force` is set.

## Recovering an uncertain command

`aggregate.command()`, `execute()`, `transaction()` and `record()` dispatch once
under an operation id: pass `{ operationId }`, or let the client generate one.
`kernel.newOperationId()` mints one before dispatch so you can persist it (with
`aggregate.capability.toWire()` and the request body) before your process can
die. The client never retries and never refreshes a handle.

Every failure carries `error.operationId` and `error.outcome`:

| `error.outcome` | Meaning | Class | Next step |
|---|---|---|---|
| `not-sent` | Provably never dispatched | `PreDispatchError` (missing `ik`) | Fix, send again with the same id |
| `refused` | Kernel answered with a diagnostic | `StaleCapabilityError`, `OperationConflictError` | Act on the diagnostic |
| `submission-unknown` | May have committed | `TransportError` after a timeout | `machine.lookupOperation(...)` |

Persist the description before sending and reconcile from it after a crash (the runnable test is
`typescript/src/invocation.test.ts`):

```ts
const invocation = aggregate.prepare({ name: "start" });
jobRecord.invocation = invocation.toJson(); // in your durable job record, before dispatch
await machine.dispatch(invocation);          // exactly once; no retry
// after a restart:
const found = await machine.reconcile(Invocation.fromJson(jobRecord.invocation));
```

```ts
const operationId = kernel.newOperationId();
const wire = aggregate.capability.toWire(); // persist both before dispatch
try {
  await aggregate.execute({ name: "start" }, { operationId });
} catch (err) {
  if (err instanceof TransportError) {
    const fresh = await new Kernel(store).machine(path);
    const found = await fresh.lookupOperation("job-1", Capability.fromWire(wire), operationId, {
      command: { name: "start" },
    });
    // found.outcome: "committed_exact" | "conflict" | "not_recorded"
    // conflict: the lookup fingerprint does not match the original request.
  }
}
```

`classify(errorOrHistory)` returns `not_sent`, `refused`,
`contract_mismatch`, `not_found`, `corrupt`, `historical`, or `uncertain`.
`historical` is a `HistoricalResult`, not an exception. `StoreNotFoundError` is
`IK_STORE_NOT_FOUND`. `error.rule` and `error.diagnostic` are preserved. See
[error taxonomy](design/error-taxonomy.md).

An exact repeat of a committed operation resolves to a `HistoricalResult`:
history plus current coordinates, no capability and no state. Call
`machine.show(id)` for a current handle. `not_recorded` is not permission to
retry under a stale handle. `replay`/`verify` audit the journal; they do not
answer whether an operation committed.

## Generated contracts (`desmo codegen`)

```bash
desmo codegen job.ik.toml --out src/job.contract.ts      # generate
desmo codegen job.ik.toml --out src/job.contract.ts --check   # exit 1 when stale
```

The generator reads `ik schema <machine>` (stable, Rust-derived metadata:
states, facts, command arguments, successor states) and never parses the
definition or evaluates a guard itself. The output is a disposable projection;
commit it only alongside a `--check` in CI. It provides `Command` (a union by
command name with typed arguments), `Facts` (a fact with no initial value is
optional: no total field is invented), `SuccessorOf<"command">` (several
possible successors stay a union), the machine `MACHINE` identity,
`assertMachine`, and a type-checked `execute(aggregate, command)`.

```ts
import { execute, command, assertMachine } from "./job.contract.js";
assertMachine(jobs);                       // throws StaleContractError if regenerated is needed
await execute(job, command({ name: "submit", args: { artifact_digest: "a", baseline_digest: "b" } }));
```

Unknown commands and malformed arguments fail `tsc`. The types are not
ownership: an aliased or copied handle, or a cast, still compiles and is refused
by the kernel at runtime (`IK_CAPABILITY_STALE`, `IK_COMMAND_INVALID`).
`typescript/examples/verification_attempt.contract.ts` is a checked-in generated
example guarded by a test.

## Recording an external effect outcome

`aggregate.pendingIntent` is the exact pending effect identity. `generation` on that object is the planned generation. It is absent when nothing is pending. `recordEffectOutcome(outcomeDigest, { operationId, intent })` copies effect id, aggregate id, planned generation, payload digest, and idempotency key from that intent. It does not invent a digest, call an external service, replace the planned generation with the current one, or retry. A transport failure after dispatch is submission-unknown. An empty digest, or an intent that is not the pending one, throws `EffectCompletionError` (`not-sent`) before dispatch.

`record()` still accepts a complete outcome object. A partial object is `ProtocolError` (`IK_PROTOCOL_INVALID`), not a transport error: the refusal echoes the request id and the pending effect stays pending. The same outcome again is a no-op. A different digest is `EffectConflictError`. A wrong planned generation or payload digest is `EffectConflictError`. Another aggregate intent is `EffectMismatchError`. Nothing is recorded in those cases.
The protocol's complete outcome object is `EffectOutcomeV1`:

```json
{
  "effect_id": "publish-1",
  "aggregate_id": "paper-1",
  "planned_generation": 1,
  "payload_digest": "artifact-digest",
  "idempotency_key": "publish-1",
  "outcome_digest": "response-digest"
}
```

The first five fields bind the result to the pending intent; `outcome_digest` is supplied by the host. The runtime binds it but does not compute it or establish that an external operation occurred. `recordEffectOutcome(outcomeDigest, { operationId, intent })` constructs this complete object from `intent` and the supplied digest. The lower-level `record()` API requires all six fields; a partial object is refused with `IK_PROTOCOL_INVALID`. For example, after the recipe has obtained `pending` and computed `outcomeDigest`:

```ts
await aggregate.record({
  effect_id: pending.effectId,
  aggregate_id: pending.aggregateId,
  planned_generation: pending.generation,
  payload_digest: pending.payloadDigest,
  idempotency_key: pending.idempotencyKey,
  outcome_digest: outcomeDigest,
});
```


A crash after the planning command commits leaves the pending intent: reopen and `show`. A crash after the external system answered, before record, is that same intent plus the digest the host saved. A dropped record reply is `lookupOperation` with the original capability, operation id, and effect body, not a second record. `finish` is a later command. A crash between record and finish leaves the outcome recorded and the domain state still `planned`.

The runnable recipe is the fenced program below. It uses `machines/effect_probe.ik.toml`. Set `STORE` and `MACHINE_PATH`. `IK_BINARY` selects `ik`.

```ts
// effect-completion-recipe
import { createHash } from "node:crypto";
import { createServer } from "node:http";
import { Kernel, Capability } from "@desmo/runtime";

const store = process.env.STORE;
const machinePath = process.env.MACHINE_PATH;
if (!store || !machinePath) {
  throw new Error("STORE and MACHINE_PATH are required");
}

const RESPONSE_BODY = '{"ok":true}';

function digestOf(body) {
  return createHash("sha256").update(body).digest("hex");
}

function effectBody(intent, outcomeDigest) {
  return {
    effect_id: intent.effectId,
    aggregate_id: intent.aggregateId,
    planned_generation: intent.generation,
    payload_digest: intent.payloadDigest,
    idempotency_key: intent.idempotencyKey,
    outcome_digest: outcomeDigest,
  };
}

function listen(body, hits) {
  const server = createServer((req, res) => {
    hits.n += 1;
    req.resume();
    res.writeHead(200, { "content-type": "application/json" });
    res.end(body);
  });
  return new Promise((resolve, reject) => {
    server.listen(0, "127.0.0.1", () => {
      const address = server.address();
      if (!address || typeof address === "string") {
        reject(new Error("fake service has no port"));
        return;
      }
      resolve({ server, url: "http://127.0.0.1:" + address.port + "/work" });
    });
  });
}

async function postWork(url, payloadDigest) {
  const response = await fetch(url, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ payload_digest: payloadDigest }),
  });
  return response.text();
}

async function openHandle(id) {
  const kernel = new Kernel(store);
  const machine = await kernel.machine(machinePath);
  const aggregate = id ? await machine.show(id) : undefined;
  return { kernel, machine, aggregate };
}

async function plan() {
  const opened = await openHandle();
  const created = await opened.machine.create("paper-1");
  const planned = await created.command("plan", {
    effect_id: "publish-1",
    payload_digest: "artifact-digest",
    idempotency_key: "publish-1",
  });
  if (!planned.pendingIntent) {
    throw new Error("plan did not leave a pending intent");
  }
  return { id: planned.id, intent: planned.pendingIntent };
}

async function main() {
  const planned = await plan();
  const afterCommit = await openHandle(planned.id);
  const pending = afterCommit.aggregate.pendingIntent;
  if (!pending || pending.effectId !== planned.intent.effectId) {
    throw new Error("crash after commit lost the pending intent");
  }
  const hits = { n: 0 };
  const service = await listen(RESPONSE_BODY, hits);
  const responseBody = await postWork(service.url, pending.payloadDigest);
  const outcomeDigest = digestOf(responseBody);
  const afterHost = await openHandle(planned.id);
  if (!afterHost.aggregate.pendingIntent) {
    throw new Error("crash after the external call lost the pending intent");
  }
  const beforeRecord = hits.n;
  const operationId = afterHost.kernel.newOperationId();
  const capability = afterHost.aggregate.capability.toWire();
  const effect = effectBody(afterHost.aggregate.pendingIntent, outcomeDigest);
  await afterHost.aggregate.recordEffectOutcome(outcomeDigest, {
    operationId,
    intent: afterHost.aggregate.pendingIntent,
  });
  if (hits.n !== beforeRecord) {
    throw new Error("recordEffectOutcome called the fake service");
  }
  service.server.close();
  const fresh = await openHandle(planned.id);
  const found = await fresh.machine.lookupOperation(
    planned.id,
    Capability.fromWire(capability),
    operationId,
    { effect },
  );
  if (found.outcome !== "committed_exact") {
    throw new Error("dropped reply was not committed_exact");
  }
  const current = await fresh.machine.show(planned.id);
  const finished = await current.command("finish", { result_digest: outcomeDigest });
  console.log(JSON.stringify({
    aggregateId: planned.id,
    responseBody,
    serviceCalls: hits.n,
    operationId,
    lookup: found.outcome,
    plannedGeneration: Number(pending.generation),
    recordedGeneration: Number(current.effect.intent.generation),
    recordedOutcome: current.effect.outcomeDigest,
    state: finished.namedState(),
  }));
}

main();
```

Repeating the same outcome is a no-op; a different `outcome_digest` for an already-recorded effect is a conflict.
The kernel binds whatever digest you pass, so record only after the external system has confirmed the result you
intend to bind. Recording advances the aggregate's generation.

## Validated admission

`create` then a required initialize is two operations, and `create` refuses an
existing `aggregateId`. `createWith` commits genesis and the initial command in
one transaction: invalid input exposes no aggregate, the same `operationId`
replays as a `HistoricalResult`, and a dropped reply is reconciled with
`lookupCreateWith(aggregateId, operationId, command)` (no capability).

Important: if an aggregate already exists with only `[genesis, initial_event]`
matching the exact same machine identity and command, `createWith` treats an
identical admission as an idempotent `ExactDuplicate` (`mutation: false`,
returning the live `Aggregate` handle) even under a **new** `operationId`. If
your domain requires refusing any duplicate admission of an existing id, check
existence first (`show`) or use a unique/distinct command payload. See
`docs/patterns.md`.
