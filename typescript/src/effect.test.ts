import assert from "node:assert/strict";
import { createServer } from "node:http";
import { chmodSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import {
  Aggregate,
  EffectIntent,
  HistoricalResult,
  Kernel,
  Machine,
} from "./client.js";
import {
  EffectCompletionError,
  EffectConflictError,
  EffectMismatchError,
  ProtocolError,
  TransportError,
} from "./errors.js";

const repo = fileURLToPath(new URL("../..", import.meta.url));
const machinePath = join(repo, "machines", "effect_probe.ik.toml");

function freshKernel(): Kernel {
  const dir = mkdtempSync(join(tmpdir(), "desmo-ts-fx-"));
  return new Kernel(join(dir, "store.db"));
}

async function planned(kernel: Kernel, id = "fx") {
  const machine = await kernel.machine(machinePath);
  const created = await machine.create(id);
  const next = await created.command("plan", {
    effect_id: "e1",
    payload_digest: "p1",
    idempotency_key: "k1",
  });
  return { machine, aggregate: next };
}

function wireOf(intent: EffectIntent, outcomeDigest: string) {
  return {
    effect_id: intent.effectId,
    aggregate_id: intent.aggregateId,
    planned_generation: intent.generation,
    payload_digest: intent.payloadDigest,
    idempotency_key: intent.idempotencyKey,
    outcome_digest: outcomeDigest,
  };
}

test("outcome is bound to the original intent, not a later generation", async () => {
  const kernel = freshKernel();
  const { aggregate } = await planned(kernel);
  const intent = aggregate.pendingIntent;
  assert.ok(intent);
  const advanced = await aggregate.command("finish", { result_digest: "later" });
  assert.notEqual(advanced.generation, intent.generation);
  assert.equal(advanced.pendingIntent?.effectId, intent.effectId);
  assert.equal(String(advanced.pendingIntent?.generation), String(intent.generation));
  const recorded = await advanced.recordEffectOutcome("o1", { intent });
  assert.ok(recorded instanceof Aggregate);
  assert.equal(recorded.effect.status, "recorded");
  assert.equal(recorded.effect.outcomeDigest, "o1");
  assert.equal(recorded.effect.intent?.effectId, intent.effectId);
  assert.equal(String(recorded.effect.intent?.generation), String(intent.generation));
  assert.equal(recorded.effect.intent?.payloadDigest, intent.payloadDigest);
});

test("repeated outcome is a no-op and a conflicting outcome leaves the first", async () => {
  const kernel = freshKernel();
  const { machine, aggregate } = await planned(kernel);
  const intent = aggregate.pendingIntent;
  assert.ok(intent);
  const first = await aggregate.recordEffectOutcome("o1", { operationId: "rec-1", intent });
  assert.ok(first instanceof Aggregate);
  assert.equal(first.mutation, true);
  const replay = await aggregate.recordEffectOutcome("o1", { operationId: "rec-1", intent });
  assert.ok(replay instanceof HistoricalResult);
  const again = await aggregate.recordEffectOutcome("o1");
  assert.ok(again instanceof Aggregate);
  assert.equal(again.mutation, false);
  await assert.rejects(
    () => first.record({ ...wireOf(intent, "other") }, { operationId: "rec-2" }),
    (err: unknown) => err instanceof EffectConflictError && err.code === "IK_EFFECT_OUTCOME_CONFLICT",
  );
  const shown = await machine.show(aggregate.id);
  assert.equal(shown.effect.outcomeDigest, "o1");
  assert.equal(shown.generation, first.generation);
});

test("wrong binding is the kernel diagnostic and records nothing", async () => {
  const kernel = freshKernel();
  const { machine, aggregate } = await planned(kernel);
  const intent = aggregate.pendingIntent;
  assert.ok(intent);
  const other = await (await kernel.machine(machinePath)).create("other");
  const otherPlanned = await other.command("plan", {
    effect_id: "e-other",
    payload_digest: "p-other",
    idempotency_key: "k-other",
  });
  assert.ok(otherPlanned.pendingIntent);
  const cases = [
    { planned_generation: Number(intent.generation) + 9, code: "IK_EFFECT_OUTCOME_CONFLICT" },
    { payload_digest: "not-the-planned-digest", code: "IK_EFFECT_OUTCOME_CONFLICT" },
    {
      effect_id: otherPlanned.pendingIntent.effectId,
      aggregate_id: otherPlanned.id,
      planned_generation: otherPlanned.pendingIntent.generation,
      payload_digest: otherPlanned.pendingIntent.payloadDigest,
      idempotency_key: otherPlanned.pendingIntent.idempotencyKey,
      code: "IK_EFFECT_MISMATCH",
    },
  ];
  for (const item of cases) {
    const { code, ...fields } = item;
    await assert.rejects(
      () => aggregate.record({ ...wireOf(intent, "o-bad"), ...fields }, { operationId: `bad-${code}` }),
      (err: unknown) => {
        if (code === "IK_EFFECT_MISMATCH") {
          return err instanceof EffectMismatchError && err.code === code;
        }
        return err instanceof EffectConflictError && err.code === code;
      },
    );
  }
  const shown = await machine.show(aggregate.id);
  assert.equal(shown.generation, aggregate.generation);
  assert.equal(shown.effect.status, "pending");
  assert.equal(shown.effect.outcomeDigest, undefined);
});

test("empty digest and a foreign intent are refused before dispatch", async () => {
  const kernel = freshKernel();
  const { machine, aggregate } = await planned(kernel);
  const intent = aggregate.pendingIntent;
  assert.ok(intent);
  const dir = mkdtempSync(join(tmpdir(), "desmo-ts-nosend-"));
  const log = join(dir, "sent.log");
  const stub = join(dir, "log-ik");
  writeFileSync(stub, `#!/bin/sh\necho sent >> "${log}"\n`);
  chmodSync(stub, 0o755);
  const quiet = new Kernel(kernel.store, { binary: stub });
  const detached = new Aggregate(
    new Machine(quiet, machine.path, machine.identity),
    aggregate.id,
    aggregate.capability,
    aggregate.state,
    aggregate.generation,
    aggregate.terminal,
    aggregate.raw,
    aggregate.mutation,
    aggregate.effect,
  );
  assert.throws(
    () => detached.recordEffectOutcome("", { operationId: "empty-1" }),
    (err: unknown) => err instanceof EffectCompletionError && err.outcome === "not-sent",
  );
  const wrong = EffectIntent.fromWire({ ...intent.toWire(), generation: Number(intent.generation) + 4 });
  assert.throws(
    () => detached.recordEffectOutcome("o1", { operationId: "wrong-intent", intent: wrong }),
    (err: unknown) => err instanceof EffectCompletionError && err.outcome === "not-sent",
  );
  const { existsSync } = await import("node:fs");
  assert.equal(existsSync(log), false);
  const shown = await machine.show(aggregate.id);
  assert.equal(shown.generation, aggregate.generation);
  const lookup = await machine.lookupOperation(aggregate.id, aggregate.capability, "empty-1", {
    effect: wireOf(intent, "unused"),
  });
  assert.equal(lookup.outcome, "not_recorded");
});

test("a partial record object is a protocol refusal, not a transport error", async () => {
  const kernel = freshKernel();
  const { machine, aggregate } = await planned(kernel);
  await assert.rejects(
    () => aggregate.record({ effect_id: "e1" }, { operationId: "partial-1" }),
    (err: unknown) => {
      assert.ok(err instanceof ProtocolError);
      assert.equal(err instanceof TransportError, false);
      assert.equal(err.code, "IK_PROTOCOL_INVALID");
      assert.equal(err.outcome, "refused");
      assert.equal(err.operationId, "partial-1");
      return true;
    },
  );
  const shown = await machine.show(aggregate.id);
  assert.equal(shown.generation, aggregate.generation);
  assert.equal(shown.effect.status, "pending");
  const lookup = await machine.lookupOperation(aggregate.id, aggregate.capability, "partial-1", {
    effect: wireOf(aggregate.pendingIntent as EffectIntent, "unused"),
  });
  assert.equal(lookup.outcome, "not_recorded");
});

test("a dropped reply is reconciled by lookup without a second record", async () => {
  const kernel = freshKernel();
  const { machine, aggregate } = await planned(kernel);
  const intent = aggregate.pendingIntent;
  assert.ok(intent);
  const operationId = "dropped-1";
  const effect = wireOf(intent, "o-dropped");
  await aggregate.recordEffectOutcome("o-dropped", { operationId, intent });
  const fresh = await new Kernel(kernel.store).machine(machinePath);
  const found = await fresh.lookupOperation(aggregate.id, aggregate.capability, operationId, { effect });
  assert.equal(found.outcome, "committed_exact");
  const shown = await fresh.show(aggregate.id);
  assert.equal(shown.effect.outcomeDigest, "o-dropped");
  assert.equal(Number(shown.generation), Number(aggregate.generation) + 1);
});

test("the helper does not call the fake service and does not retry a lost transport", async () => {
  const kernel = freshKernel();
  const { machine, aggregate } = await planned(kernel);
  const intent = aggregate.pendingIntent;
  assert.ok(intent);
  let calls = 0;
  const server = createServer((_req, res) => {
    calls += 1;
    res.end("nope");
  });
  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", () => resolve()));
  await aggregate.recordEffectOutcome("o-local", { intent });
  assert.equal(calls, 0);
  server.close();
  const dir = mkdtempSync(join(tmpdir(), "desmo-ts-slow-"));
  const log = join(dir, "dispatches.log");
  const stub = join(dir, "slow-ik");
  writeFileSync(stub, `#!/bin/sh\necho dispatched >> "${log}"\nsleep 5\n`);
  chmodSync(stub, 0o755);
  const slow = new Kernel(kernel.store, { binary: stub, timeoutMs: 300 });
  const slowAggregate = new Aggregate(
    new Machine(slow, machine.path, machine.identity),
    aggregate.id,
    aggregate.capability,
    aggregate.state,
    aggregate.generation,
    aggregate.terminal,
    aggregate.raw,
    aggregate.mutation,
    aggregate.effect,
  );
  await assert.rejects(
    () => slowAggregate.recordEffectOutcome("o-lost", { operationId: "maybe-recorded", intent }),
    (err: unknown) => err instanceof TransportError && err.outcome === "submission-unknown",
  );
  const { readFileSync } = await import("node:fs");
  assert.deepEqual(readFileSync(log, "utf8").trim().split("\n"), ["dispatched"]);
});
