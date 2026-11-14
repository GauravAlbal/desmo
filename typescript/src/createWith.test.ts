import assert from "node:assert/strict";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { packageRoot } from "./assets.js";
import { HistoricalResult, Kernel } from "./client.js";
import { KernelError } from "./errors.js";

const machinePath = join(packageRoot(), "../examples/approval-revision/approval_revision.ik.toml");
const EDIT = { name: "edit", args: { artifact_digest: "doc-1" } };
const BAD = { name: "approve", args: { revision: 1 } };

function kernel(): Kernel {
  const dir = mkdtempSync(join(tmpdir(), "desmo-create-with-"));
  return new Kernel(join(dir, "store.db"));
}

test("invalid initial command exposes no aggregate", async () => {
  const jobs = await kernel().machine(machinePath);
  await assert.rejects(() => jobs.createWith({ aggregateId: "doc-bad", command: BAD, operationId: "op-bad" }), KernelError);
  await assert.rejects(() => jobs.show("doc-bad"), KernelError);
  const lookup = await jobs.lookupCreateWith("doc-bad", "op-bad", BAD);
  assert.equal(lookup.outcome, "not_recorded");
});

test("the same operation id replays history and lookup finds the commit", async () => {
  const jobs = await kernel().machine(machinePath);
  const created = await jobs.createWith({ aggregateId: "doc-good", command: EDIT, operationId: "op-good" });
  assert.ok(!(created instanceof HistoricalResult));
  assert.equal(created.namedState(), "draft");
  assert.equal(created.generation, 1);
  const again = await jobs.createWith({ aggregateId: "doc-good", command: EDIT, operationId: "op-good" });
  assert.ok(again instanceof HistoricalResult);
  assert.equal(again.kind, "create_with_command");
  const lookup = await jobs.lookupCreateWith("doc-good", "op-good", EDIT);
  assert.equal(lookup.outcome, "committed_exact");
  assert.equal(lookup.history?.kind, "create_with_command");
});
