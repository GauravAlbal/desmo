import assert from "node:assert/strict";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import { bundledMachine } from "./assets.js";
import { Kernel } from "./client.js";
import { KernelError, StaleCapabilityError, continuation } from "./errors.js";
import { renderTraceHtml, type TraceBundle } from "./trace.js";

const machine = bundledMachine();

test("stale worker cannot finish after cancel", async () => {
  const dir = mkdtempSync(join(tmpdir(), "desmo-ts-"));
  const kernel = new Kernel(join(dir, "store.db"));
  const jobs = await kernel.machine(machine);
  const job = await jobs.create("example");
  const running = await job.command("start");
  const stale = running;
  const cancelled = await running.command("cancel");
  assert.equal(cancelled.namedState(), "cancelled");
  await assert.rejects(() => stale.command("submit", {
    artifact_digest: "a",
    baseline_digest: "b",
  }), (err: unknown) => {
    assert.ok(err instanceof StaleCapabilityError);
    if (!(err instanceof StaleCapabilityError)) return false;
    assert.equal(err.code, "IK_CAPABILITY_STALE");
    assert.match(continuation(err), /Do not automatically resubmit/);
    return true;
  });
  const shown = await jobs.show("example");
  assert.equal(shown.namedState(), "cancelled");
  assert.equal(shown.generation, cancelled.generation);
});

test("trace html escapes payloads and states that a refused destination was never reached", () => {
  const bundle: TraceBundle = {
    schema: "ikaros.trace.v2",
    product: "Ikaros",
    runtime_version: "0.6.0",
    machine_id: "verification_attempt",
    advisory: true,
    origin: "observed",
    reproducible: false,
    redacted: false,
    events: [
      { kind: "commit", attempted: "cancel", fromState: "running", toState: "cancelled", generation: "2" },
      {
        kind: "refusal",
        attempted: "<script>finish</script>",
        fromState: "cancelled",
        intendedState: "verified",
        code: "IK_CAPABILITY_STALE",
        message: "stale",
      },
    ],
  };
  const html = renderTraceHtml(bundle);
  assert.equal(html.includes("<script>finish"), false);
  assert.equal(html.includes("&lt;script&gt;finish"), true);
  assert.equal(html.includes("never reached verified"), true);
  assert.equal(html.includes("Advisory"), true);
});

test("owner names the definition that owns an old aggregate", async () => {
  const machines = (name: string) => fileURLToPath(new URL(`../../machines/${name}`, import.meta.url));
  const dir = mkdtempSync(join(tmpdir(), "desmo-ts-owner-"));
  const kernel = new Kernel(join(dir, "store.db"));
  const v1 = await kernel.machine(machines("publish_approval.ik.toml"));
  const v2 = await kernel.machine(machines("publish_approval_v2.ik.toml"));
  await v1.create("legacy-doc");
  await v2.create("fresh-doc");
  const legacy = await v2.show("legacy-doc");
  assert.ok(legacy.owner.equals(v1.identity));
  assert.ok(!legacy.owner.equals(v2.identity));
  assert.ok((await v2.show("fresh-doc")).owner.equals(v2.identity));
  const routed = [v1, v2].find((m) => m.identity.equals(legacy.owner));
  assert.equal(routed, v1);
});

test("text_in identifier guard commits a valid id and refuses an empty id", async () => {
  const tagged = fileURLToPath(new URL("../../examples/shipped/tagged_item.ik.toml", import.meta.url));
  const repo = fileURLToPath(new URL("../..", import.meta.url));
  const binary = process.env.IK_BINARY || process.env.IK_BIN || join(repo, "target", "debug", "ik");
  const dir = mkdtempSync(join(tmpdir(), "desmo-ts-text-in-"));
  const kernel = new Kernel(join(dir, "store.db"), { binary });
  const items = await kernel.machine(tagged);
  const taggedItem = await (await items.create("item-1")).command("tag", { id: "item_1", label: "AMBER" });
  assert.equal(taggedItem.namedState(), "tagged");
  assert.equal(taggedItem.terminal, true);
  const empty = await items.create("item-empty");
  await assert.rejects(() => empty.command("tag", { id: "", label: "AMBER" }), (err: unknown) => {
    assert.ok(err instanceof KernelError);
    assert.equal((err as KernelError).code, "IK_INVARIANT_FAILED");
    assert.equal((err as KernelError).rule, "tag.when");
    return true;
  });
  const shown = await items.show("item-empty");
  assert.equal(shown.namedState(), "idle");
});
