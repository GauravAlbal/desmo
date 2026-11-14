import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { copyFileSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { test } from "node:test";
import { fileURLToPath, pathToFileURL } from "node:url";
import { bundledMachine } from "./assets.js";
import { SchemaError, generateContracts, parseMachineSchema } from "./codegen.js";
import { Kernel } from "./client.js";
import { parseWire } from "./json.js";
import { KernelError, StaleCapabilityError } from "./errors.js";
import { locateBinary } from "./locate.js";

const root = fileURLToPath(new URL("..", import.meta.url));
const tscBin = join(root, "node_modules", "typescript", "bin", "tsc");
const cliJs = join(root, "dist", "cli.js");
const runtimeIndex = join(root, "dist", "index.js");
const machinePath = bundledMachine();

function schemaText(path: string): string {
  return execFileSync(locateBinary(), ["schema", path], { encoding: "utf8" });
}

function generated(path: string): string {
  return generateContracts(parseMachineSchema(parseWire(schemaText(path))), {
    importFrom: runtimeIndex,
  });
}

/** Compile `files` in `dir` (inside the package so node_modules types resolve). */
function tsc(dir: string, files: string[], emit: boolean): { status: number; output: string } {
  const args = [
    tscBin,
    ...(emit ? ["--outDir", join(dir, "out")] : ["--noEmit"]),
    "--strict",
    "--target", "es2022",
    "--module", "node16",
    "--moduleResolution", "node16",
    "--skipLibCheck",
    "--types", "node",
    ...files,
  ];
  try {
    const output = execFileSync(process.execPath, args, { cwd: dir, encoding: "utf8", stdio: "pipe" });
    return { status: 0, output };
  } catch (err) {
    const failure = err as { status?: number; stdout?: string };
    return { status: failure.status ?? -1, output: failure.stdout ?? "" };
  }
}

function workDir(): string {
  return mkdtempSync(join(root, ".codegen-"));
}

test("generated contracts compile for valid scenarios and expose conditional successors as unions", () => {
  const dir = workDir();
  try {
    writeFileSync(join(dir, "contract.ts"), generated(machinePath));
    writeFileSync(
      join(dir, "scenario.ts"),
      `import { command, type SuccessorOf, type Facts, type StateName } from "./contract.js";
export const submit = command({ name: "submit", args: { artifact_digest: "a", baseline_digest: "b" } });
export const start = command({ name: "start" });
export const rejected: SuccessorOf<"reject"> = "exhausted";
export const other: SuccessorOf<"reject"> = "repairable";
export const state: StateName = "running";
export const facts: Facts = { attempt_count: 0n, max_attempts: 3 };
`,
    );
    const result = tsc(dir, ["contract.ts", "scenario.ts"], false);
    assert.equal(result.status, 0, result.output);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("unknown commands, missing or wrong arguments, and a narrowed conditional outcome fail the TypeScript compiler", () => {
  const dir = workDir();
  try {
    writeFileSync(join(dir, "contract.ts"), generated(machinePath));
    const cases: Record<string, string> = {
      "unknown command": `command({ name: "explode" });`,
      "missing argument": `command({ name: "submit", args: { artifact_digest: "a" } });`,
      "wrong argument type": `command({ name: "submit", args: { artifact_digest: 5, baseline_digest: "b" } });`,
      "arguments on an argument-free command": `command({ name: "start", args: { x: "y" } });`,
      "conditional outcome narrowed to one successor":
        `declare const s: SuccessorOf<"reject">; export const only: "repairable" = s;`,
      "unknown successor": `export const bad: SuccessorOf<"start"> = "verified";`,
      "required fact omitted": `export const f: Facts = { max_attempts: 3 };`,
    };
    for (const [name, body] of Object.entries(cases)) {
      writeFileSync(
        join(dir, "negative.ts"),
        `import { command, type SuccessorOf, type Facts } from "./contract.js";\n${body}\n`,
      );
      const result = tsc(dir, ["contract.ts", "negative.ts"], false);
      assert.notEqual(result.status, 0, `${name} must not compile`);
      assert.match(result.output, /negative\.ts/, name);
    }
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("compiled contracts run against the kernel; aliased handles and casts are still refused at runtime", async () => {
  const dir = workDir();
  try {
    writeFileSync(join(dir, "contract.ts"), generated(machinePath));
    const built = tsc(dir, ["contract.ts"], true);
    assert.equal(built.status, 0, built.output);
    const contract = await import(pathToFileURL(join(dir, "out", "contract.js")).href);

    const kernel = new Kernel(join(dir, "store.db"));
    const jobs = await kernel.machine(machinePath);
    contract.assertMachine(jobs);
    const created = await jobs.create("job-1");
    const alias = created;
    const started = await contract.execute(created, contract.command({ name: "start" }));
    assert.equal(started.namedState(), "running");

    // Statically valid, but the handle is stale: the kernel refuses at runtime.
    await assert.rejects(
      () => contract.execute(alias, contract.command({ name: "cancel" })),
      StaleCapabilityError,
    );
    // An unsafe cast bypasses host typing only; the kernel still refuses an unknown command.
    await assert.rejects(
      () => contract.execute(started, { name: "explode" } as never),
      (err: unknown) => {
        assert.ok(err instanceof KernelError);
        assert.equal(err.code, "IK_COMMAND_INVALID");
        return true;
      },
    );
    // And it cannot skip a currentness/state rule: submit is illegal before... it is legal here, so cancel twice.
    const cancelled = await contract.execute(started, contract.command({ name: "cancel" }));
    await assert.rejects(
      () => contract.execute(cancelled, contract.command({ name: "start" })),
      KernelError,
    );
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("a contract generated from a different machine identity is rejected at runtime", async () => {
  const dir = workDir();
  try {
    const original = schemaText(machinePath);
    const drifted = original.replace(/"spec_hash":"[0-9a-f]+"/, `"spec_hash":"${"0".repeat(64)}"`);
    assert.notEqual(drifted, original);
    const source = generateContracts(parseMachineSchema(parseWire(drifted)), { importFrom: runtimeIndex });
    writeFileSync(join(dir, "contract.ts"), source);
    const built = tsc(dir, ["contract.ts"], true);
    assert.equal(built.status, 0, built.output);
    const contract = await import(pathToFileURL(join(dir, "out", "contract.js")).href);
    const jobs = await new Kernel(join(dir, "store.db")).machine(machinePath);
    assert.throws(() => contract.assertMachine(jobs), /generated contract is for/);
    const created = await jobs.create("job-1");
    await assert.rejects(
      () => contract.execute(created, contract.command({ name: "start" })),
      (err: unknown) => err instanceof Error && err.name === "StaleContractError",
    );
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("generation is deterministic, and --check fails once the definition changes", () => {
  const dir = workDir();
  try {
    const machine = join(dir, "job.ik.toml");
    copyFileSync(machinePath, machine);
    const out = join(dir, "contract.ts");
    const run = (...args: string[]): { status: number; text: string } => {
      try {
        const text = execFileSync(process.execPath, [cliJs, "codegen", machine, "--out", out, ...args], {
          encoding: "utf8",
          stdio: "pipe",
        });
        return { status: 0, text };
      } catch (err) {
        const failure = err as { status?: number; stderr?: string };
        return { status: failure.status ?? -1, text: failure.stderr ?? "" };
      }
    };
    assert.equal(run().status, 0);
    const first = readFileSync(out, "utf8");
    assert.equal(run().status, 0);
    assert.equal(readFileSync(out, "utf8"), first, "same definition regenerates identical bytes");
    assert.equal(run("--check").status, 0);

    const edited = readFileSync(machine, "utf8").replace("[states]", "[states]\nextra = {}");
    assert.notEqual(edited, readFileSync(machinePath, "utf8"));
    writeFileSync(machine, edited);
    const stale = run("--check");
    assert.equal(stale.status, 1);
    assert.match(stale.text, /stale or missing/);
    assert.equal(run().status, 0);
    assert.notEqual(readFileSync(out, "utf8"), first, "a definition change regenerates different output");
    assert.equal(run("--check").status, 0);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("a malformed or foreign schema document is rejected, not guessed at", () => {
  assert.throws(() => parseMachineSchema({ schema: "something.else" }), SchemaError);
  assert.throws(
    () =>
      parseMachineSchema({
        schema: "ikaros.machine-schema.v1",
        machine: { machine_id: "m", version: 1, spec_hash: "h" },
        states: [],
        facts: [{ name: "x", type: "float", policy: "replace", required: true }],
        commands: [],
        invariants: [],
      }),
    /unsupported type/,
  );
});

test("the checked-in generated example matches the current machine definition", () => {
  const machine = join(root, "..", "machines", "verification_attempt.ik.toml");
  const expected = generateContracts(parseMachineSchema(parseWire(schemaText(machine))), {
    importFrom: "@desmo/runtime",
  });
  const checkedIn = readFileSync(join(root, "examples", "verification_attempt.contract.ts"), "utf8");
  assert.equal(
    checkedIn,
    expected,
    "regenerate with: desmo codegen machines/verification_attempt.ik.toml --out typescript/examples/verification_attempt.contract.ts",
  );
});
