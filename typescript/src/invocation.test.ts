import assert from "node:assert/strict";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { bundledMachine } from "./assets.js";
import type { Json } from "./client.js";
import {
  Aggregate,
  Capability,
  HistoricalResult,
  Invocation,
  Kernel,
  MachineIdentity,
} from "./client.js";

const machinePath = bundledMachine();
const START = { name: "start" };

function freshKernel(): Kernel {
  const dir = mkdtempSync(join(tmpdir(), "desmo-ts-inv-"));
  return new Kernel(join(dir, "store.db"));
}

test("a crash after commit is reconciled from the persisted description", async () => {
  const kernel = freshKernel();
  const jobs = await kernel.machine(machinePath);
  const created = await jobs.create("job-1");
  const invocation = created.prepare(START);
  const persisted = invocation.toJson();
  const started = await jobs.dispatch(invocation);
  assert.ok(started instanceof Aggregate);
  const moved = await started.command("cancel");
  assert.equal(moved.namedState(), "cancelled");

  const fresh = await new Kernel(kernel.store).machine(machinePath);
  const lookup = await fresh.reconcile(Invocation.fromJson(persisted));
  assert.equal(lookup.outcome, "committed_exact");
  assert.equal(lookup.history?.generation, 1);
  assert.equal(lookup.history?.currentGeneration, 2);
});

test("a prepared but unsent invocation is not recorded and can still be dispatched", async () => {
  const kernel = freshKernel();
  const jobs = await kernel.machine(machinePath);
  const created = await jobs.create("job-2");
  const invocation = created.prepare(START);
  assert.equal((await jobs.reconcile(invocation)).outcome, "not_recorded");
  assert.ok((await jobs.dispatch(Invocation.fromJson(invocation.toJson()))) instanceof Aggregate);
  assert.equal((await jobs.reconcile(invocation)).outcome, "committed_exact");
});

test("dispatching the same invocation again is history, not a second commit", async () => {
  const kernel = freshKernel();
  const jobs = await kernel.machine(machinePath);
  const created = await jobs.create("job-3");
  const invocation = created.prepare(START);
  assert.ok((await jobs.dispatch(invocation)) instanceof Aggregate);
  const repeat = await jobs.dispatch(Invocation.fromJson(invocation.toJson()));
  assert.ok(repeat instanceof HistoricalResult);
  assert.equal(repeat.operationId, invocation.operationId);
  assert.equal((await jobs.show("job-3")).generation, 1);
});

test("a refreshed capability is a different request and conflicts", async () => {
  const kernel = freshKernel();
  const jobs = await kernel.machine(machinePath);
  const created = await jobs.create("job-4");
  const invocation = created.prepare(START);
  await jobs.dispatch(invocation);
  const refreshed = await jobs.show("job-4");
  assert.notEqual(refreshed.prepare(START).operationId, invocation.operationId);
  const lookup = await jobs.lookupOperation("job-4", refreshed.capability, invocation.operationId, {
    command: START,
  });
  assert.equal(lookup.outcome, "conflict");
});

test("a transaction invocation round-trips and reconciles", async () => {
  const kernel = freshKernel();
  const jobs = await kernel.machine(machinePath);
  const created = await jobs.create("job-5");
  const invocation = created.prepareTransaction([{ command: START }]);
  const wire = invocation.toWire();
  assert.equal(wire.kind, "transaction");
  assert.equal((await jobs.reconcile(Invocation.fromWire(wire))).outcome, "not_recorded");
  assert.ok((await jobs.dispatch(invocation)) instanceof Aggregate);
  assert.equal((await jobs.reconcile(Invocation.fromWire(wire))).outcome, "committed_exact");
});

test("an invocation for another machine identity is refused", async () => {
  const kernel = freshKernel();
  const jobs = await kernel.machine(machinePath);
  const created = await jobs.create("job-6");
  const wire = created.prepare(START).toWire();
  wire.machine = { ...(wire.machine as Record<string, string>), spec_hash: "0".repeat(64) };
  const other = Invocation.fromWire(wire);
  await assert.rejects(() => jobs.dispatch(other), /different machine identity/);
  await assert.rejects(() => jobs.reconcile(other), /different machine identity/);
});

const GOOD = {
  invocation_version: 1,
  kind: "command",
  machine: { machine_id: "m", version: 1, spec_hash: "h" },
  aggregate_id: "a",
  operation_id: "op",
  capability: { generation: 0, nonce: "n" },
  command: { name: "start" },
};

test("the wire form rejects unknown versions, fields and empty ids", () => {
  assert.throws(() => Invocation.fromWire({ invocation_version: 2 }), /unknown invocation_version/);
  assert.equal(Invocation.fromWire(GOOD).operationId, "op");
  assert.throws(() => Invocation.fromWire({ ...GOOD, surprise: 1 }), /unknown invocation fields/);
  assert.throws(() => Invocation.fromWire({ ...GOOD, operation_id: "" }), /non-empty operation id/);
  assert.throws(() => Invocation.fromWire({ ...GOOD, kind: "transaction" }), /transaction invocation/);
});

test("a pinned fixture serializes to the same bytes in every client", () => {
  const invocation = new Invocation(
    "command",
    new MachineIdentity("m", 1, "h"),
    "a",
    "op",
    new Capability({ generation: 0, nonce: "n" }),
    { command: { name: "start" } },
  );
  assert.equal(
    invocation.toJson(),
    '{"aggregate_id":"a","capability":{"generation":0,"nonce":"n"},"command":{"name":"start"},"invocation_version":1,"kind":"command","machine":{"machine_id":"m","spec_hash":"h","version":1},"operation_id":"op"}',
  );
});

test("the prepared request is a snapshot of what was persisted", async () => {
  const kernel = freshKernel();
  const jobs = await kernel.machine(machinePath);
  const started = await jobs.dispatch((await jobs.create("job-7")).prepare(START));
  assert.ok(started instanceof Aggregate);
  const args: Record<string, string> = { artifact_digest: "AMBER", baseline_digest: "b" };
  const observation = { note: { tags: ["x"] } };
  const invocation = started.prepare({ name: "submit", args }, { observation });
  const persisted = invocation.toJson();

  args.artifact_digest = "BLUE";
  observation.note.tags.push("y");
  const exported = invocation.toWire();
  ((exported.command as Record<string, Json>).args as Record<string, Json>).artifact_digest = "RED";
  ((invocation.body.command as Record<string, Json>).args as Record<string, Json>).artifact_digest = "GREEN";
  assert.equal(invocation.toJson(), persisted);

  const sent = await jobs.dispatch(invocation);
  assert.ok(sent instanceof Aggregate);
  const moved = await sent.command("cancel");
  assert.ok(BigInt(moved.generation) > BigInt(sent.generation));
  const fresh = await new Kernel(kernel.store).machine(machinePath);
  assert.equal((await fresh.reconcile(Invocation.fromJson(persisted))).outcome, "committed_exact");
});

test("transaction steps are snapshotted and unserializable input is refused", async () => {
  const kernel = freshKernel();
  const jobs = await kernel.machine(machinePath);
  const created = await jobs.create("job-8");
  const steps: Record<string, Json>[] = [{ command: { name: "start" } }, { command: { name: "cancel" } }];
  const invocation = created.prepareTransaction(steps);
  const persisted = invocation.toJson();
  (steps[1].command as Record<string, Json>).name = "start";
  steps.push({ command: { name: "start" } });
  assert.equal(invocation.toJson(), persisted);
  assert.ok((await jobs.dispatch(invocation)) instanceof Aggregate);
  const fresh = await new Kernel(kernel.store).machine(machinePath);
  assert.equal((await fresh.reconcile(Invocation.fromJson(persisted))).outcome, "committed_exact");
  assert.throws(() => created.prepare({ name: "start", args: { n: 9007199254740993 } }), /lossy|safe|integer/i);
});

const CORPUS =
  '{"aggregate_id":"a","capability":{"generation":0,"nonce":"n"},"command":{"args":{"label":"\u00e9<&\u2603","n":9007199254740993,"nested":{"a":[1,{"b":"x"}]}},"name":"start"},"invocation_version":1,"kind":"command","machine":{"machine_id":"m","spec_hash":"h","version":1},"operation_id":"op"}';

test("the shared corpus restores with exact values and survives a second round trip", () => {
  const first = Invocation.fromJson(CORPUS);
  const args = (first.body.command as Record<string, Json>).args as Record<string, Json>;
  assert.equal(args.label, "\u00e9<&\u2603");
  assert.equal(args.n, 9007199254740993n);
  const again = Invocation.fromJson(first.toJson());
  assert.deepEqual(again.toWire(), first.toWire());
  assert.ok(again.toJson().includes("9007199254740993"));
});
