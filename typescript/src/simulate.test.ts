import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import { bundledOrderMachine } from "./assets.js";
import type { MachineSchema } from "./codegen.js";
import { renderSimulatorHtml } from "./simulate.js";

test("renderSimulatorHtml produces self-contained advisory HTML and escapes hostile strings", () => {
  const hostile: MachineSchema = {
    machine: {
      machine_id: 'evil</script><script>alert("xss")</script>',
      version: "1",
      spec_hash: "a".repeat(64),
    },
    states: [
      { name: "start<img src=x>", initial: true, terminal: false },
      { name: "done", initial: false, terminal: true },
    ],
    facts: [{ name: "note", type: "string", policy: "replace", required: false }],
    commands: [
      {
        name: "finish",
        from: ["start<img src=x>"],
        args: [],
        successors: ["done"],
        conditional_successor: false,
        plans_effect: false,
      },
    ],
    invariants: ["inv_1"],
  };
  const html = renderSimulatorHtml(hostile);
  assert.match(html, /Advisory structural simulation/);
  assert.match(html, /IK_ILLEGAL_STATE/);
  assert.match(html, /IK_CAPABILITY_STALE/);
  assert.equal(html.includes("</script><script>alert"), false);
  assert.equal(html.includes("<img src=x>"), false);
});

test("desmo simulate writes a standalone interactive HTML simulator from a real .ik.toml", () => {
  const dir = mkdtempSync(join(tmpdir(), "desmo-sim-"));
  try {
    const out = join(dir, "order-sim.html");
    const cliJs = join(dirname(fileURLToPath(import.meta.url)), "cli.js");
    const res = spawnSync(
      process.execPath,
      [cliJs, "simulate", bundledOrderMachine(), "--out", out],
      { encoding: "utf8" },
    );
    assert.equal(res.status, 0, res.stderr);
    assert.match(res.stdout, /order v1, 6 states, 5 commands/);
    const html = readFileSync(out, "utf8");
    assert.match(html, /<!DOCTYPE html>/);
    assert.match(html, /"machine_id":"order"/);
    assert.match(html, /Copy Capability Handle/);
    assert.match(html, /Fire Stale Handle/);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});
