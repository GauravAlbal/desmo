import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawnSync } from "node:child_process";
import { test } from "node:test";
import { fileURLToPath, pathToFileURL } from "node:url";
import { Kernel } from "./client.js";
import { locateBinary } from "./locate.js";

const repo = fileURLToPath(new URL("../..", import.meta.url));
const runtime = pathToFileURL(fileURLToPath(new URL("./index.js", import.meta.url))).href;
const machinePath = join(repo, "machines", "effect_probe.ik.toml");

function recipeSource(): string {
  const docs = readFileSync(join(repo, "docs", "typescript.md"), "utf8");
  const match = docs.match(/```ts\n\/\/ effect-completion-recipe\n([\s\S]*?)```/);
  assert.ok(match, "docs/typescript.md is missing the effect-completion recipe");
  return match[1];
}

test("the typescript recipe runs against ik and a local fake service", async () => {
  const source = recipeSource().replace(
    'from "@desmo/runtime"',
    `from ${JSON.stringify(runtime)}`,
  );
  const dir = mkdtempSync(join(tmpdir(), "desmo-ts-recipe-"));
  const script = join(dir, "recipe.mjs");
  const store = join(dir, "store.db");
  writeFileSync(script, source);
  const result = spawnSync(process.execPath, [script], {
    env: { ...process.env, STORE: store, MACHINE_PATH: machinePath, IK_BINARY: locateBinary() },
    encoding: "utf8",
  });
  assert.equal(result.status, 0, result.stderr || result.stdout);
  const summary = JSON.parse(result.stdout.trim().split("\n").at(-1) ?? "");
  const expectedDigest = createHash("sha256").update(summary.responseBody).digest("hex");
  assert.equal(summary.serviceCalls, 1);
  assert.equal(summary.lookup, "committed_exact");
  assert.equal(summary.plannedGeneration, 1);
  assert.equal(summary.recordedGeneration, 1);
  assert.equal(summary.recordedOutcome, expectedDigest);
  assert.equal(summary.state, "finished");
  const shown = await new Kernel(store).machine(machinePath).then((machine) => machine.show(summary.aggregateId));
  assert.equal(shown.namedState(), "finished");
  assert.equal(shown.effect.status, "recorded");
  assert.equal(shown.effect.outcomeDigest, expectedDigest);
  assert.equal(String(shown.effect.intent?.generation), "1");
  assert.equal(shown.effect.intent?.effectId, "publish-1");
});
