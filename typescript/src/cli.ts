#!/usr/bin/env node
import { execFileSync } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { bundledMachine } from "./assets.js";
import { Kernel } from "./client.js";
import { continuation, StaleCapabilityError } from "./errors.js";
import { parseTraceBundle, renderTraceHtml, type TraceBundle, type TraceEvent } from "./trace.js";
import { generateContracts, parseMachineSchema } from "./codegen.js";
import { parseWire } from "./json.js";
import { locateBinary } from "./locate.js";
import { inspectProject, packageVersion, renderDoctor } from "./doctor.js";
import {
  DESMO_USAGE,
  applyInstall,
  packagedSkillDir,
  planInstall,
  renderInstall,
  unknownSkillFlag,
} from "./skill.js";
import { WORKBENCH_README, writeJobWorkbench } from "./scaffold.js";

type NaiveJob = { state: string };

function naiveLateWorkerOverwrite(): string {
  const job: NaiveJob = { state: "running" };
  job.state = "cancelled";
  job.state = "done";
  return job.state;
}

async function demoStaleWorker(): Promise<void> {
  console.log("Executed naïve baseline (in-process map, no kernel):");
  console.log(`  cancel then late write → ${naiveLateWorkerOverwrite()}\n`);

  const dir = mkdtempSync(join(tmpdir(), "desmo-demo-"));
  const machine = bundledMachine();
  const kernel = new Kernel(join(dir, "store.db"));
  const jobs = await kernel.machine(machine);
  const job = await jobs.create("example");
  const running = await job.command("start");
  const cancelled = await running.command("cancel");
  const events: TraceEvent[] = [
    {
      kind: "commit",
      attempted: "cancel",
      fromState: "running",
      toState: cancelled.namedState(),
      generation: String(cancelled.generation),
    },
  ];
  try {
    await running.command("submit", { artifact_digest: "late", baseline_digest: "worker" });
    console.log("unexpected commit");
    process.exitCode = 1;
    return;
  } catch (err) {
    if (!(err instanceof StaleCapabilityError)) {
      throw err;
    }
    const fresh = await jobs.show("example");
    console.log(`finish refused (${err.code})`);
    console.log(`This worker holds a stale capability.`);
    console.log(`Fresh read: generation ${String(fresh.generation)}: ${fresh.namedState()}.`);
    console.log(continuation(err));
    events.push({
      kind: "refusal",
      attempted: "submit",
      fromState: cancelled.namedState(),
      intendedState: "submitted",
      generation: String(fresh.generation),
      code: err.code,
      message: err.message,
      continuation: continuation(err),
    });
    events.push({
      kind: "read",
      attempted: "show",
      state: fresh.namedState(),
      generation: String(fresh.generation),
      message: "fresh durable read after refusal",
    });
  }
  const bundle: TraceBundle = {
    schema: "ikaros.trace.v2",
    product: "Ikaros",
    runtime_version: "0.5.0",
    machine_id: "verification_attempt",
    advisory: true,
    origin: "observed",
    reproducible: true,
    redacted: false,
    events,
    reproduction: "npx --no-install desmo demo stale-worker",
  };
  const htmlPath = join(dir, "trace.html");
  writeFileSync(htmlPath, renderTraceHtml(bundle));
  writeFileSync(
    join(dir, "trace.json"),
    JSON.stringify(bundle, (_k, v) => (typeof v === "bigint" ? v.toString() : v), 2),
  );
  console.log(`\nTrace: ${htmlPath}`);
}

function flagValue(argv: string[], name: string): string | undefined {
  const i = argv.indexOf(name);
  if (i === -1) {
    return undefined;
  }
  const value = argv[i + 1];
  if (value === undefined || value.startsWith("--")) {
    throw new Error(`${name} requires a value`);
  }
  return value;
}

function initJob(target: string): void {
  if (existsSync(target) && readdirSync(target).length > 0) {
    throw new Error(`refusing to overwrite non-empty directory ${target}`);
  }
  mkdirSync(target, { recursive: true });
  writeFileSync(
    join(target, "package.json"),
    JSON.stringify(
      {
        name: "desmo-job",
        private: true,
        type: "module",
        scripts: {
          start: "node job.mjs",
          test: "node --test job.test.mjs",
        },
        dependencies: { "@desmo/runtime": "0.5.0" },
      },
      null,
      2,
    ),
  );
  writeFileSync(
    join(target, "README.md"),
    `# Desmo job template

One machine definition in \`job.ik.toml\`. Runtime remains authoritative.

    npm test
    npm start
` + WORKBENCH_README,
  );
  writeJobWorkbench(target);
  writeFileSync(
    join(target, "job.mjs"),
    `import { Kernel } from "@desmo/runtime";

const kernel = new Kernel(".state/kernel.db");
const jobs = await kernel.machine("job.ik.toml");
const job = await jobs.create("job-1");
const running = await job.command("start");
console.log(running.namedState());
`,
  );
  writeFileSync(
    join(target, "job.test.mjs"),
    `import assert from "node:assert/strict";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { Kernel, StaleCapabilityError } from "@desmo/runtime";

test("start commits", async () => {
  const dir = mkdtempSync(join(tmpdir(), "desmo-job-"));
  const jobs = await new Kernel(join(dir, "store.db")).machine("job.ik.toml");
  const job = await jobs.create("job-1");
  const running = await job.command("start");
  assert.equal(running.namedState(), "running");
});

test("cancel then submit is refused; fresh read stays cancelled", async () => {
  const dir = mkdtempSync(join(tmpdir(), "desmo-job-"));
  const jobs = await new Kernel(join(dir, "store.db")).machine("job.ik.toml");
  const job = await jobs.create("job-1");
  const running = await job.command("start");
  await running.command("cancel");
  await assert.rejects(() => running.command("submit", {
    artifact_digest: "a",
    baseline_digest: "b",
  }), StaleCapabilityError);
  const fresh = await jobs.show("job-1");
  assert.equal(fresh.namedState(), "cancelled");
});
`,
  );
  copyFileSync(bundledMachine(), join(target, "job.ik.toml"));
  console.log(`wrote ${target}`);
}

/** `desmo codegen`: generate (or, with --check, verify) TypeScript contracts from `ik schema`. */
function codegen(args: string[]): void {
  const machinePath = args[0];
  if (!machinePath || machinePath.startsWith("--")) {
    throw new Error("usage: desmo codegen <machine.ik.toml> [--out file] [--check] [--import-from specifier]");
  }
  const out = flagValue(args, "--out");
  const importFrom = flagValue(args, "--import-from") ?? "@desmo/runtime";
  const raw = execFileSync(locateBinary(), ["schema", machinePath], { encoding: "utf8" });
  const source = generateContracts(parseMachineSchema(parseWire(raw)), { importFrom });
  if (args.includes("--check")) {
    if (!out) {
      throw new Error("--check requires --out <generated file>");
    }
    if (!existsSync(out) || readFileSync(out, "utf8") !== source) {
      console.error(`${out} is stale or missing: regenerate with \`desmo codegen ${machinePath} --out ${out}\``);
      process.exitCode = 1;
      return;
    }
    console.log(`${out} is up to date`);
    return;
  }
  if (out) {
    writeFileSync(out, source);
    console.log(`wrote ${out}`);
  } else {
    process.stdout.write(source);
  }
}

/** `desmo trace`: validate a trace bundle and render it to standalone HTML. Advisory only. */
function traceCommand(args: string[]): void {
  const bundlePath = args[0];
  const out = flagValue(args, "--out");
  if (!bundlePath || bundlePath.startsWith("--") || !out) {
    throw new Error("usage: desmo trace <bundle.json> --out <file.html>");
  }
  const bundle = parseTraceBundle(parseWire(readFileSync(bundlePath, "utf8")));
  writeFileSync(out, renderTraceHtml(bundle));
  console.log(`wrote ${out} (${bundle.origin}, ${bundle.events.length} events)`);
}


function skillCommand(args: string[]): void {
  if (args.length === 0 || args.includes("-h") || args.includes("--help")) {
    console.log(DESMO_USAGE);
    return;
  }
  if (args[0] !== "install") {
    console.error(DESMO_USAGE);
    process.exitCode = 1;
    return;
  }
  const unknown = unknownSkillFlag(args.slice(1));
  if (unknown) {
    throw new Error(`unknown flag ${unknown}`);
  }
  const plan = planInstall(packagedSkillDir(), {
    projectDir: flagValue(args, "--dir") ?? process.cwd(),
    targetDir: flagValue(args, "--target"),
    force: args.includes("--force"),
  });
  applyInstall(packagedSkillDir(), plan);
  process.stdout.write(renderInstall(plan));
  if (plan.conflicts.length > 0) {
    process.exitCode = 1;
  }
}

function doctorCommand(args: string[]): void {
  const dir = flagValue(args, "--dir") ?? process.cwd();
  const json = args.includes("--json");
  let binary: string;
  try {
    binary = locateBinary();
  } catch (err) {
    console.error(`ik runtime not found: ${err instanceof Error ? err.message : err}`);
    process.exitCode = 1;
    return;
  }
  let versionLine: string;
  try {
    versionLine = execFileSync(binary, ["--version"], { encoding: "utf8" });
  } catch (err) {
    console.error(`ik runtime not found: ${err instanceof Error ? err.message : err}`);
    process.exitCode = 1;
    return;
  }
  const report = inspectProject(dir, binary, versionLine, packageVersion());
  if (json) {
    console.log(JSON.stringify(report, null, 2));
    return;
  }
  process.stdout.write(renderDoctor(report));
}

const argv = process.argv.slice(2);
try {
  if (argv[0] === "demo" && argv[1] === "stale-worker") {
    await demoStaleWorker();
  } else if (argv[0] === "codegen") {
    codegen(argv.slice(1));
  } else if (argv[0] === "trace") {
    traceCommand(argv.slice(1));
  } else if (argv[0] === "doctor") {
    doctorCommand(argv.slice(1));
  } else if (argv[0] === "skill") {
    skillCommand(argv.slice(1));
  } else if (argv[0] === "init") {
    const lang = flagValue(argv, "--language") ?? "typescript";
    const template = flagValue(argv, "--template") ?? "job";
    if (lang !== "typescript" || template !== "job") {
      console.error("only --template job --language typescript is supported at launch");
      process.exitCode = 1;
    } else {
      initJob(flagValue(argv, "--out") ?? "desmo-job");
    }
  } else {
    console.log(DESMO_USAGE);
  }
} catch (err) {
  console.error(err instanceof Error ? err.message : err);
  process.exitCode = 1;
}
