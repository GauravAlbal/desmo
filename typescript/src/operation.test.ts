import assert from "node:assert/strict";
import { chmodSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { bundledMachine } from "./assets.js";
import {
  Aggregate,
  Capability,
  HistoricalResult,
  Kernel,
  Machine,
} from "./client.js";
import {
  KernelError,
  OperationConflictError,
  PreDispatchError,
  StaleCapabilityError,
  TransportError,
  continuation,
} from "./errors.js";

const machinePath = bundledMachine();
const START = { name: "start" };
const CANCEL = { name: "cancel" };

function freshKernel(): Kernel {
  const dir = mkdtempSync(join(tmpdir(), "desmo-ts-op-"));
  return new Kernel(join(dir, "store.db"));
}

test("dropped reply then another writer is recovered by a fresh client", async () => {
  const kernel = freshKernel();
  const jobs = await kernel.machine(machinePath);
  const created = await jobs.create("job-1");
  const persistedCapability = JSON.stringify(created.capability.toWire());
  const operationId = kernel.newOperationId();
  // The submitting process commits, then dies before reading the reply.
  const started = await created.execute(START, { operationId });
  assert.ok(started instanceof Aggregate);
  // A different writer advances the aggregate past the original capability.
  const cancelled = await started.command("cancel");
  assert.equal(cancelled.namedState(), "cancelled");

  const freshJobs = await new Kernel(kernel.store).machine(machinePath);
  const capability = Capability.fromWire(JSON.parse(persistedCapability));
  const lookup = await freshJobs.lookupOperation("job-1", capability, operationId, {
    command: START,
  });
  assert.equal(lookup.outcome, "committed_exact");
  assert.equal(lookup.history?.firstSequence, 1);
  assert.equal(lookup.history?.generation, 1);
  assert.equal(lookup.history?.currentGeneration, 2);
});

test("an exact repeat is history without authority and mutates nothing", async () => {
  const kernel = freshKernel();
  const jobs = await kernel.machine(machinePath);
  const created = await jobs.create("job-1");
  const started = await created.execute(START, { operationId: "repeat-1" });
  assert.ok(started instanceof Aggregate);
  await started.command("cancel");

  const repeat = await created.execute(START, { operationId: "repeat-1" });
  assert.ok(repeat instanceof HistoricalResult);
  assert.equal(Object.hasOwn(repeat, "capability"), false);
  assert.equal(repeat.operationId, "repeat-1");
  assert.equal(repeat.currentGeneration, 2);
  const after = await jobs.show("job-1");
  assert.equal(after.generation, 2);
  assert.equal(after.namedState(), "cancelled");
});

test("a stale handle under a different operation id is still refused and carries its id", async () => {
  const kernel = freshKernel();
  const jobs = await kernel.machine(machinePath);
  const created = await jobs.create("job-1");
  await created.execute(START, { operationId: "first-op" });
  await assert.rejects(
    () => created.execute(CANCEL, { operationId: "different-op" }),
    (err: unknown) => {
      assert.ok(err instanceof StaleCapabilityError);
      assert.equal(err.outcome, "refused");
      assert.equal(err.operationId, "different-op");
      return true;
    },
  );
});

test("the same operation id with changed semantics conflicts", async () => {
  const kernel = freshKernel();
  const jobs = await kernel.machine(machinePath);
  const created = await jobs.create("job-1");
  await created.execute(START, { operationId: "reused" });
  await assert.rejects(
    () => created.execute(CANCEL, { operationId: "reused" }),
    (err: unknown) => {
      assert.ok(err instanceof OperationConflictError);
      assert.equal(err.code, "IK_OPERATION_CONFLICT");
      assert.equal(err.operationId, "reused");
      return true;
    },
  );
  const lookup = await jobs.lookupOperation("job-1", created.capability, "reused", {
    command: CANCEL,
  });
  assert.equal(lookup.outcome, "conflict");
});

test("lookup of an unsent operation is not recorded", async () => {
  const kernel = freshKernel();
  const jobs = await kernel.machine(machinePath);
  const created = await jobs.create("job-1");
  const lookup = await jobs.lookupOperation("job-1", created.capability, "never-sent", {
    command: START,
  });
  assert.equal(lookup.outcome, "not_recorded");
  assert.equal(lookup.history, undefined);
  assert.equal((await jobs.show("job-1")).generation, 0);
});

test("a generated operation id is exposed on the error and absent from the journal after a refusal", async () => {
  const kernel = freshKernel();
  const jobs = await kernel.machine(machinePath);
  const created = await jobs.create("job-1");
  const illegal = { name: "submit", args: { artifact_digest: "a", baseline_digest: "b" } };
  let generated = "";
  await assert.rejects(
    () => created.execute(illegal),
    (err: unknown) => {
      assert.ok(err instanceof KernelError);
      generated = err.operationId ?? "";
      assert.match(generated, /^ts-/);
      return true;
    },
  );
  const lookup = await jobs.lookupOperation("job-1", created.capability, generated, {
    command: illegal,
  });
  assert.equal(lookup.outcome, "not_recorded");
});

test("a transaction exact repeat is history over its sequence range", async () => {
  const kernel = freshKernel();
  const jobs = await kernel.machine(machinePath);
  const created = await jobs.create("job-1");
  const first = await created.transaction([{ command: START }, { command: CANCEL }], {
    operationId: "batch-1",
  });
  assert.ok(first instanceof Aggregate);
  const repeat = await created.transaction([{ command: START }, { command: CANCEL }], {
    operationId: "batch-1",
  });
  assert.ok(repeat instanceof HistoricalResult);
  assert.equal(repeat.kind, "transaction");
  assert.equal(repeat.firstSequence, 1);
  assert.equal(repeat.lastSequence, 2);
});

test("a missing binary is pre-dispatch, keeps the operation id, and commits nothing", async () => {
  const kernel = freshKernel();
  const jobs = await kernel.machine(machinePath);
  const created = await jobs.create("job-1");
  const dir = mkdtempSync(join(tmpdir(), "desmo-ts-missing-"));
  const missing = new Kernel(kernel.store, { binary: join(dir, "no-such-ik") });
  const detached = new Aggregate(
    new Machine(missing, jobs.path, jobs.identity),
    created.id,
    created.capability,
    created.state,
    created.generation,
    created.terminal,
    created.raw,
    created.mutation,
  );
  await assert.rejects(
    () => detached.execute(START, { operationId: "persisted-before-dispatch" }),
    (err: unknown) => {
      assert.ok(err instanceof PreDispatchError);
      assert.equal(err.outcome, "not-sent");
      assert.equal(err.operationId, "persisted-before-dispatch");
      assert.match(continuation(err), /send again with the same operation id/);
      return true;
    },
  );
  const lookup = await jobs.lookupOperation("job-1", created.capability, "persisted-before-dispatch", {
    command: START,
  });
  assert.equal(lookup.outcome, "not_recorded");
});

test("a timeout after dispatch is submission-unknown and is never retried", async () => {
  const kernel = freshKernel();
  const jobs = await kernel.machine(machinePath);
  const created = await jobs.create("job-1");
  const dir = mkdtempSync(join(tmpdir(), "desmo-ts-slow-"));
  const log = join(dir, "dispatches.log");
  const stub = join(dir, "slow-ik");
  writeFileSync(stub, `#!/bin/sh\necho dispatched >> "${log}"\nsleep 5\n`);
  chmodSync(stub, 0o755);
  const slow = new Kernel(kernel.store, { binary: stub, timeoutMs: 300 });
  const slowAggregate = new Aggregate(
    new Machine(slow, jobs.path, jobs.identity),
    created.id,
    created.capability,
    created.state,
    created.generation,
    created.terminal,
    created.raw,
    created.mutation,
  );
  await assert.rejects(
    () => slowAggregate.execute(START, { operationId: "maybe-committed" }),
    (err: unknown) => {
      assert.ok(err instanceof TransportError);
      assert.equal(err instanceof PreDispatchError, false);
      assert.equal(err.outcome, "submission-unknown");
      assert.equal(err.operationId, "maybe-committed");
      assert.match(continuation(err), /lookupOperation/);
      return true;
    },
  );
  assert.deepEqual(readFileSync(log, "utf8").trim().split("\n"), ["dispatched"]);
});
