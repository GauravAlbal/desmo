import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { chmodSync, existsSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import { bundledMachine, bundledOrderMachine } from "./assets.js";
import {
  Aggregate,
  Capability,
  HistoricalResult,
  Kernel,
  Machine,
  classify,
} from "./client.js";
import {
  IllegalStateError,
  KernelError,
  MachineError,
  PreDispatchError,
  StaleCapabilityError,
  StoreNotFoundError,
  TransportError,
} from "./errors.js";

const machinePath = bundledMachine();
const foreignPath = join(
  dirname(fileURLToPath(import.meta.url)),
  "..",
  "..",
  "machines",
  "effect_probe.ik.toml",
);
const START = { name: "start" };
const CANCEL = { name: "cancel" };

function kernelAt(store: string, binary?: string, timeoutMs?: number): Kernel {
  return new Kernel(store, { binary, timeoutMs });
}

test("a missing store is not_found and read-only calls create nothing", async () => {
  const parent = join(mkdtempSync(join(tmpdir(), "desmo-absent-")), "absent-parent");
  const store = join(parent, "store.db");
  assert.equal(existsSync(parent), false);
  const jobs = await kernelAt(store).machine(machinePath);
  await assert.rejects(() => jobs.show("missing"), (err: unknown) => {
    assert.ok(err instanceof StoreNotFoundError);
    assert.equal(err.code, "IK_STORE_NOT_FOUND");
    assert.equal(err.rule, "store.missing");
    assert.equal(err.diagnostic.rule, "store.missing");
    assert.equal(classify(err), "not_found");
    return true;
  });
  await assert.rejects(() => jobs.replay("missing"), (err: unknown) => {
    assert.ok(err instanceof StoreNotFoundError);
    assert.equal(classify(err), "not_found");
    return true;
  });
  await assert.rejects(() => jobs.verify("missing"), (err: unknown) => {
    assert.ok(err instanceof StoreNotFoundError);
    assert.equal(classify(err), "not_found");
    return true;
  });
  const capability = Capability.fromWire({ generation: 0, nonce: "n" });
  await assert.rejects(
    () => jobs.lookupOperation("missing", capability, "op-1", { command: START }),
    (err: unknown) => {
      assert.ok(err instanceof StoreNotFoundError);
      assert.equal(err.code, "IK_STORE_NOT_FOUND");
      assert.equal(classify(err), "not_found");
      return true;
    },
  );
  assert.equal(existsSync(store), false);
  assert.equal(existsSync(parent), false);
});

test("create is the operation that creates a store file", async () => {
  const dir = mkdtempSync(join(tmpdir(), "desmo-create-"));
  const store = join(dir, "created.db");
  assert.equal(existsSync(store), false);
  const jobs = await kernelAt(store).machine(machinePath);
  await jobs.create("agg-1");
  assert.equal(existsSync(store), true);
});

test("a garbage store file is corrupt, not missing", async () => {
  const dir = mkdtempSync(join(tmpdir(), "desmo-garbage-"));
  const store = join(dir, "garbage.db");
  writeFileSync(store, "this is not a sqlite database");
  const jobs = await kernelAt(store).machine(machinePath);
  await assert.rejects(() => jobs.show("agg-1"), (err: unknown) => {
    assert.ok(err instanceof KernelError);
    assert.equal(err instanceof StoreNotFoundError, false);
    assert.equal(err.code, "IK_STORE_CORRUPT");
    assert.notEqual(err.rule, "store.missing");
    assert.equal(classify(err), "corrupt");
    return true;
  });
  assert.equal(existsSync(store), true);
  await assert.rejects(() => jobs.show("agg-1"), (err: unknown) => {
    assert.ok(err instanceof KernelError);
    assert.equal(err.code, "IK_STORE_CORRUPT");
    assert.equal(classify(err), "corrupt");
    return true;
  });
});

test("an unknown aggregate is not_found, not corrupt", async () => {
  const dir = mkdtempSync(join(tmpdir(), "desmo-absent-agg-"));
  const jobs = await kernelAt(join(dir, "store.db")).machine(machinePath);
  await jobs.create("present");
  await assert.rejects(() => jobs.show("absent"), (err: unknown) => {
    assert.ok(err instanceof KernelError);
    assert.equal(err.code, "IK_ILLEGAL_STATE");
    assert.equal(err.rule, "aggregate.not_found");
    assert.equal(err.diagnostic.rule, "aggregate.not_found");
    assert.equal(classify(err), "not_found");
    assert.notEqual(classify(err), "corrupt");
    return true;
  });
});

test("a stale capability is refused", async () => {
  const dir = mkdtempSync(join(tmpdir(), "desmo-stale-"));
  const jobs = await kernelAt(join(dir, "store.db")).machine(machinePath);
  const created = await jobs.create("attempt-1");
  await created.execute(START, { operationId: "first-op" });
  await assert.rejects(
    () => created.execute(CANCEL, { operationId: "other-op" }),
    (err: unknown) => {
      assert.ok(err instanceof StaleCapabilityError);
      assert.equal(err.code, "IK_CAPABILITY_STALE");
      assert.equal(err.outcome, "refused");
      assert.equal(classify(err), "refused");
      return true;
    },
  );
});

test("an exact repeat is historical, not an error", async () => {
  const dir = mkdtempSync(join(tmpdir(), "desmo-hist-"));
  const jobs = await kernelAt(join(dir, "store.db")).machine(machinePath);
  const created = await jobs.create("attempt-1");
  await created.execute(START, { operationId: "same-op" });
  const repeat = await created.execute(START, { operationId: "same-op" });
  assert.ok(repeat instanceof HistoricalResult);
  assert.equal(repeat instanceof KernelError, false);
  assert.equal(classify(repeat), "historical");
});

test("a missing binary is not_sent", async () => {
  const dir = mkdtempSync(join(tmpdir(), "desmo-unsent-"));
  const jobs = await kernelAt(join(dir, "store.db")).machine(machinePath);
  const created = await jobs.create("attempt-1");
  const missing = kernelAt(jobs.kernel.store, join(dir, "no-such-ik"));
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
    () => detached.execute(START, { operationId: "never-left" }),
    (err: unknown) => {
      assert.ok(err instanceof PreDispatchError);
      assert.equal(err.outcome, "not-sent");
      assert.equal(err.operationId, "never-left");
      assert.equal(classify(err), "not_sent");
      return true;
    },
  );
});

test("a timeout after dispatch is uncertain", async () => {
  const dir = mkdtempSync(join(tmpdir(), "desmo-uncertain-"));
  const jobs = await kernelAt(join(dir, "store.db")).machine(machinePath);
  const created = await jobs.create("attempt-1");
  const log = join(dir, "dispatches.log");
  const stub = join(dir, "slow-ik");
  writeFileSync(stub, `#!/bin/sh\necho dispatched >> "${log}"\nsleep 5\n`);
  chmodSync(stub, 0o755);
  const slow = kernelAt(jobs.kernel.store, stub, 300);
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
      assert.equal(classify(err), "uncertain");
      return true;
    },
  );
  assert.deepEqual(readFileSync(log, "utf8").trim().split("\n"), ["dispatched"]);
});

test("a legacy missing-store code with the store.missing rule is not_found", () => {
  const legacy = new KernelError("IK_STORE_CORRUPT", "store file missing", {
    code: "IK_STORE_CORRUPT",
    rule: "store.missing",
    message: "store file missing",
  });
  assert.equal(legacy.rule, "store.missing");
  assert.equal(classify(legacy), "not_found");
});

test("a machine mismatch is contract_mismatch", async () => {
  const dir = mkdtempSync(join(tmpdir(), "desmo-mismatch-"));
  const kernel = kernelAt(join(dir, "store.db"));
  const home = await kernel.machine(machinePath);
  const created = await home.create("attempt-1");
  const foreign = await kernel.machine(foreignPath);
  const mismatched = new Aggregate(
    foreign,
    created.id,
    created.capability,
    created.state,
    created.generation,
    created.terminal,
    created.raw,
    created.mutation,
  );
  await assert.rejects(
    () => mismatched.execute(START, { operationId: "wrong-machine" }),
    (err: unknown) => {
      assert.ok(err instanceof KernelError);
      assert.equal(err.code.startsWith("IK_MACHINE_"), true);
      assert.equal(classify(err), "contract_mismatch");
      return true;
    },
  );
});

test("the same operation id on two aggregates does not collide", async () => {
  const dir = mkdtempSync(join(tmpdir(), "desmo-scope-"));
  const jobs = await kernelAt(join(dir, "store.db")).machine(machinePath);
  const first = await jobs.create("agg-a");
  const second = await jobs.create("agg-b");
  const startedA = await first.execute(START, { operationId: "shared-op" });
  const startedB = await second.execute(START, { operationId: "shared-op" });
  assert.ok(startedA instanceof Aggregate);
  assert.ok(startedB instanceof Aggregate);
  const lookupA = await jobs.lookupOperation("agg-a", first.capability, "shared-op", {
    command: START,
  });
  const lookupB = await jobs.lookupOperation("agg-b", second.capability, "shared-op", {
    command: START,
  });
  assert.equal(lookupA.outcome, "committed_exact");
  assert.equal(lookupB.outcome, "committed_exact");
  await assert.rejects(
    () => first.execute(CANCEL, { operationId: "shared-op" }),
    (err: unknown) => {
      assert.ok(err instanceof KernelError);
      assert.equal(err.code, "IK_OPERATION_CONFLICT");
      assert.equal(classify(err), "refused");
      return true;
    },
  );
  const stillB = await jobs.lookupOperation("agg-b", second.capability, "shared-op", {
    command: START,
  });
  assert.equal(stillB.outcome, "committed_exact");
});

test("typed error subclasses match illegal state and machine mismatch", async () => {
  const dir = mkdtempSync(join(tmpdir(), "desmo-typed-err-"));
  const store = join(dir, "orders.db");
  const orders = await kernelAt(store).machine(bundledOrderMachine());
  const order = await orders.create("order-1");
  await assert.rejects(() => order.command("refund"), (err: unknown) => {
    assert.ok(err instanceof IllegalStateError);
    assert.ok(err instanceof KernelError);
    assert.equal(err.code, "IK_ILLEGAL_STATE");
    assert.equal(classify(err), "refused");
    return true;
  });
  const foreign = await kernelAt(store).machine(foreignPath);
  const wrongMachine = new Aggregate(
    foreign,
    order.id,
    order.capability,
    order.state,
    order.generation,
    order.terminal,
    order.raw,
    order.mutation,
  );
  await assert.rejects(() => wrongMachine.command("plan"), (err: unknown) => {
    assert.ok(err instanceof MachineError);
    assert.equal(classify(err), "contract_mismatch");
    return true;
  });
});

test("desmo demo orders succeeds and unknown subcommand exits 1", () => {
  const cliJs = join(dirname(fileURLToPath(import.meta.url)), "cli.js");
  const demo = spawnSync(process.execPath, [cliJs, "demo", "orders"], { encoding: "utf8" });
  assert.equal(demo.status, 0, demo.stderr);
  assert.match(demo.stdout, /IK_ILLEGAL_STATE/);
  assert.match(demo.stdout, /committed \(refunded/);
  const bogus = spawnSync(process.execPath, [cliJs, "does-not-exist"], { encoding: "utf8" });
  assert.equal(bogus.status, 1);
  assert.match(bogus.stderr, /unknown command/);
});
