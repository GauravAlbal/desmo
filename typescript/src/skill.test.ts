import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { chmodSync, existsSync, mkdtempSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { basename, dirname, join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import { bundledMachine } from "./assets.js";
import { acceptedFlags, extractInvocations, extractRepoPaths, referenceProblems, type Invocation } from "./skill-refs.js";
import { SKILL_FILES } from "./skill.js";

const packageRoot = fileURLToPath(new URL("..", import.meta.url));
const repo = join(packageRoot, "..");
const cliJs = join(packageRoot, "dist", "cli.js");
const skillDir = join(repo, "skills", "desmo");

type Run = { status: number | null; stdout: string; stderr: string };

function ikBinary(): string {
  return process.env.IK_BINARY || process.env.IK_BIN || join(repo, "target", "debug", "ik");
}

function runCli(args: string[], cwd: string, env: NodeJS.ProcessEnv = process.env): Run {
  const result = spawnSync(process.execPath, [cliJs, ...args], { cwd, env, encoding: "utf8" });
  return { status: result.status, stdout: result.stdout ?? "", stderr: result.stderr ?? "" };
}

function runIk(args: string[], cwd: string, input?: string): Run {
  const result = spawnSync(ikBinary(), args, { cwd, encoding: "utf8", input });
  return { status: result.status, stdout: result.stdout ?? "", stderr: result.stderr ?? "" };
}

function usageText(command: string, args: string[]): string {
  const result = spawnSync(command, args, { encoding: "utf8" });
  return `${result.stdout ?? ""}\n${result.stderr ?? ""}`;
}

function usageFor(invocation: Invocation): string {
  if (invocation.binary === "desmo") {
    const chunks = [usageText(process.execPath, [cliJs]), usageText(process.execPath, [cliJs, "--help"])];
    if (invocation.words[0]) {
      chunks.push(usageText(process.execPath, [cliJs, invocation.words[0], "--help"]));
    }
    return chunks.join("\n");
  }
  const bin = ikBinary();
  const chunks = [usageText(bin, ["--help"]), usageText(bin, ["--help-all"])];
  if (invocation.words[0]) {
    chunks.push(usageText(bin, [invocation.words[0], "--help"]));
    chunks.push(usageText(bin, [invocation.words[0]]));
  }
  return chunks.join("\n");
}

function skillMarkdown(): string[] {
  return readdirSync(skillDir).filter((name) => name.endsWith(".md")).sort();
}

test("skill install is idempotent and refuses a local edit without force", () => {
  const dir = mkdtempSync(join(tmpdir(), "desmo-skill-install-"));
  try {
    const first = runCli(["skill", "install", "--dir", dir], dir);
    assert.equal(first.status, 0, first.stderr);
    const installed = join(dir, ".agents", "skills", "desmo");
    assert.match(first.stdout, new RegExp(`installed ${installed}`));
    assert.match(first.stdout, /next: point the coding agent at /);
    assert.match(first.stdout, new RegExp(`next: desmo doctor --dir ${dir}`));
    assert.match(first.stdout, /next: open SKILL.md and follow audit.md, build.md, change.md, or diagnose.md/);
    for (const name of SKILL_FILES) {
      assert.equal(readFileSync(join(installed, name), "utf8"), readFileSync(join(skillDir, name), "utf8"));
    }
    const again = runCli(["skill", "install", "--dir", dir], dir);
    assert.equal(again.status, 0, again.stderr);
    assert.match(again.stdout, /^unchanged /);
    const skillFile = join(installed, "SKILL.md");
    const edited = `${readFileSync(skillFile, "utf8")}\nlocal note\n`;
    writeFileSync(skillFile, edited);
    const refused = runCli(["skill", "install", "--dir", dir], dir);
    assert.equal(refused.status, 1, refused.stdout);
    assert.match(refused.stdout, /refusing to overwrite /);
    assert.match(refused.stdout, /--force/);
    assert.equal(readFileSync(skillFile, "utf8"), edited);
    const forced = runCli(["skill", "install", "--dir", dir, "--force"], dir);
    assert.equal(forced.status, 0, forced.stderr);
    assert.equal(readFileSync(skillFile, "utf8"), readFileSync(join(skillDir, "SKILL.md"), "utf8"));
    const target = join(dir, "custom-skills");
    const targeted = runCli(["skill", "install", "--dir", dir, "--target", target], dir);
    assert.equal(targeted.status, 0, targeted.stderr);
    assert.equal(existsSync(join(target, "desmo", "routing.json")), true);
    const fakeIk = join(dir, "fake-ik");
    writeFileSync(fakeIk, "#!/bin/sh\nprintf '%s\\n' 'invariant-kernel 0.7.1 protocol 1.0'\n");
    chmodSync(fakeIk, 0o755);
    const env = { ...process.env, IK_BINARY: fakeIk, IK_BIN: "" };
    const doctor = runCli(["doctor", "--dir", dir, "--json"], dir, env);
    assert.equal(doctor.status, 0, doctor.stderr);
    const report = JSON.parse(doctor.stdout) as { skill: { installed: boolean; complete: boolean; path: string } };
    assert.equal(report.skill.installed, true);
    assert.equal(report.skill.complete, true);
    assert.equal(report.skill.path, join(".agents", "skills", "desmo"));
    const text = runCli(["doctor", "--dir", dir], dir, env);
    assert.match(text.stdout, /skill: \.agents\/skills\/desmo present/);
    const empty = mkdtempSync(join(tmpdir(), "desmo-skill-absent-"));
    const absent = runCli(["doctor", "--dir", empty, "--json"], empty, env);
    assert.equal((JSON.parse(absent.stdout) as { skill: { installed: boolean } }).skill.installed, false);
    rmSync(join(installed, "routing.json"));
    const incomplete = runCli(["doctor", "--dir", dir], dir, env);
    assert.match(incomplete.stdout, /skill: \.agents\/skills\/desmo incomplete/);
    rmSync(empty, { recursive: true, force: true });
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("every cited command flag exists and a renamed flag is detected", () => {
  const problems: string[] = [];
  const paths = new Set<string>();
  let invocations = 0;
  for (const name of skillMarkdown()) {
    const text = readFileSync(join(skillDir, name), "utf8");
    for (const path of extractRepoPaths(text)) {
      paths.add(path);
    }
    for (const invocation of extractInvocations(text, name)) {
      invocations += 1;
      problems.push(...referenceProblems(invocation, usageFor(invocation)));
    }
  }
  assert.ok(invocations > 10, "extractor found no commands");
  assert.equal(problems.length, 0, problems.join("\n"));
  assert.ok(paths.has("docs/agent.md"));
  assert.ok(paths.has("docs/scenarios.md"));
  for (const path of paths) {
    assert.equal(existsSync(join(repo, path)), true, path);
  }
  const renamed = readFileSync(join(skillDir, "build.md"), "utf8").replaceAll("--profile", "--profyle");
  const renamedProblems = extractInvocations(renamed, "build.md").flatMap((invocation) =>
    referenceProblems(invocation, usageFor(invocation)),
  );
  assert.ok(renamedProblems.some((problem) => problem.includes("--profyle")));
  assert.equal(acceptedFlags("There is no --store flag.\nusage: ik scenario run --out <dir>").has("--store"), false);
  assert.equal(acceptedFlags("usage: ik scenario run --out <dir>").has("--out"), true);
});

type StoreSnap = { bytes: Buffer; mtimeMs: number; siblings: Map<string, Buffer> };
type Op = {
  id: string;
  class: string;
  bin: string;
  argv: string[];
  stdin?: string;
  touches_store: boolean;
  refuses_store_flag?: boolean;
};

function snapStore(path: string): StoreSnap {
  const dir = dirname(path);
  const base = basename(path);
  const siblings = new Map<string, Buffer>();
  for (const name of readdirSync(dir)) {
    if (name === base || name.startsWith(`${base}-`)) {
      siblings.set(name, readFileSync(join(dir, name)));
    }
  }
  return { bytes: readFileSync(path), mtimeMs: statSync(path).mtimeMs, siblings };
}

function sameBytes(left: StoreSnap, right: StoreSnap): boolean {
  if (!left.bytes.equals(right.bytes) || left.siblings.size !== right.siblings.size) {
    return false;
  }
  for (const [name, bytes] of left.siblings) {
    const other = right.siblings.get(name);
    if (!other || !other.equals(bytes)) {
      return false;
    }
  }
  return true;
}

function envelope(op: string, identity: unknown, extra: Record<string, unknown>): string {
  return JSON.stringify({
    protocol: { major: 1, minor: 0 },
    runtime_version: "0.7.1",
    request_id: `r-${op}`,
    op,
    machine: identity,
    ...extra,
  });
}

test("investigate_only operations do not change a store and scenario uses scratch", () => {
  const dir = mkdtempSync(join(tmpdir(), "desmo-routing-"));
  try {
    const machine = join(dir, "machine.ik.toml");
    writeFileSync(machine, readFileSync(bundledMachine()));
    const store = join(dir, "store.sqlite");
    const identityRun = runIk(["identity", machine], dir);
    assert.equal(identityRun.status, 0, identityRun.stderr);
    const identity: unknown = JSON.parse(identityRun.stdout);
    const created = runIk(
      ["--store", store, "--machine", machine, "create"],
      dir,
      envelope("create", identity, { aggregate_id: "job-1" }),
    );
    assert.equal(created.status, 0, created.stderr + created.stdout);
    const capability = (JSON.parse(created.stdout) as { result: { capability: unknown } }).result.capability;
    const profile = join(dir, "checks.toml");
    writeFileSync(profile, ["profile_version = 1", "", "[[goals]]", 'id = "done"', 'kind = "reach_terminal"', 'terminal = "verified"', "required = false", ""].join("\n"));
    const manifest = join(dir, "adapter.json");
    writeFileSync(manifest, JSON.stringify({
      protocol: "ik.analysis.adapter.v1",
      adapter: { id: "local", version: "1" },
      invocation: { argv: ["/bin/echo"], executable_sha256: "a".repeat(64) },
      engine: { id: "none", version_constraint: "0" },
      inputs: ["observation_trace"],
      properties: ["safety"],
      evidence: ["observed_trace"],
      requires: { production_store_access: false, network: false },
      result_schema: "ik.analysis.result.v1",
    }));
    const scenario = join(dir, "scenario.json");
    writeFileSync(scenario, JSON.stringify({
      scenario_version: 1,
      machine: "machine.ik.toml",
      aggregate: "paper-1",
      steps: [{ kind: "command", label: "start", name: "start", args: {}, expect: "committed" }],
    }));
    const bundle = join(dir, "bundle");
    const seeded = runIk(["scenario", "run", machine, scenario, "--out", bundle], dir);
    assert.equal(seeded.status, 0, seeded.stderr + seeded.stdout);
    assert.equal(existsSync(join(bundle, "scratch", "store.sqlite")), true);
    const table = JSON.parse(readFileSync(join(skillDir, "routing.json"), "utf8")) as {
      modes: Record<string, { allowed: string[]; operations?: Op[] }>;
    };
    assert.deepEqual(table.modes.investigate_only.allowed, ["read", "scratch"]);
    for (const mode of ["investigate_only", "authorized_repair", "build", "change"]) {
      assert.ok(Array.isArray(table.modes[mode].allowed), mode);
    }
    const ops = table.modes.investigate_only.operations ?? [];
    assert.ok(ops.some((op) => op.id === "diagram"));
    assert.ok(ops.some((op) => op.id === "scenario-run"));
    assert.ok(ops.some((op) => op.stdin === "lookup_operation"));
    assert.equal(ops.some((op) => op.id === "replay"), false);
    assert.equal(ops.some((op) => op.argv[0] === "analysis" && op.argv[1] === "run"), false);
    const values: Record<string, string> = {
      "{machine}": machine,
      "{store}": store,
      "{profile}": profile,
      "{manifest}": manifest,
      "{scenario}": scenario,
      "{bundle}": bundle,
      "{scratch}": join(dir, "op-scratch"),
      "{scratch_repro}": join(dir, "op-repro"),
      "{evidence}": join(bundle, "result.json"),
      "{report_input}": join(bundle, "result.json"),
      "{graph_out}": join(dir, "graph.json"),
      "{simulator_out}": join(dir, "simulator.html"),
      "{project}": dir,
    };
    const bodies: Record<string, string> = {
      show: envelope("show", identity, { aggregate_id: "job-1" }),
      replay: envelope("replay", identity, { aggregate_id: "job-1" }),
      verify: envelope("verify", identity, { aggregate_id: "job-1" }),
      lookup_operation: envelope("lookup_operation", identity, {
        aggregate_id: "job-1",
        capability,
        operation_id: "never-sent",
        command: { name: "start", args: {} },
      }),
    };
    const beforeWrites = snapStore(store);
    for (const op of ops) {
      assert.ok(op.class === "read" || op.class === "scratch", op.id);
      const mutating = ["create", "command", "record", "transaction"];
      assert.equal(mutating.some((name) => op.argv.includes(name)), false, op.id);
      const argv = op.argv.map((token) => values[token] ?? token);
      const snap = snapStore(store);
      const result = op.bin === "desmo"
        ? runCli(argv, dir, { ...process.env, IK_BINARY: ikBinary() })
        : runIk(argv, dir, op.stdin ? bodies[op.stdin] : undefined);
      assert.equal(result.status, 0, `${op.id}\n${result.stderr}\n${result.stdout}`);
      const after = snapStore(store);
      assert.equal(sameBytes(snap, after), true, `${op.id} changed store bytes`);
      if (!op.touches_store) {
        assert.equal(after.mtimeMs, snap.mtimeMs, `${op.id} changed store mtime`);
      }
      if (op.id === "scenario-run") {
        const scratchStore = join(values["{scratch}"], "scratch", "store.sqlite");
        assert.equal(existsSync(scratchStore), true);
        assert.notEqual(scratchStore, store);
      }
      if (op.id === "graph-export") {
        assert.equal(existsSync(values["{graph_out}"]), true);
        assert.notEqual(values["{graph_out}"], store);
      }
      if (op.id === "simulate") {
        assert.equal(existsSync(values["{simulator_out}"]), true);
        assert.notEqual(values["{simulator_out}"], store);
      }
      if (op.refuses_store_flag) {
        const refused = runIk([...argv, "--store", store], dir);
        assert.notEqual(refused.status, 0, op.id);
        assert.match(refused.stderr, /--store/);
        assert.equal(sameBytes(after, snapStore(store)), true, `${op.id} store flag wrote the store`);
      }
    }
    const mutated = runIk(
      ["--store", store, "--machine", machine, "command"],
      dir,
      envelope("command", identity, {
        aggregate_id: "job-1",
        capability,
        operation_id: "start-1",
        command: { name: "start", args: {} },
      }),
    );
    assert.equal(mutated.status, 0, mutated.stderr + mutated.stdout);
    assert.equal(sameBytes(beforeWrites, snapStore(store)), false);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("build and change skills encode identifier grammars with text_in", () => {
  const build = readFileSync(join(skillDir, "build.md"), "utf8");
  const change = readFileSync(join(skillDir, "change.md"), "utf8");
  assert.match(build, /text_in\(\$arg, 'A-Za-z0-9\._-', 1, 64\)/);
  assert.match(change, /text_in\(\$arg, 'A-Za-z0-9\._-', 1, 64\)/);
  assert.equal(build.includes("has no string-length or character-class predicate"), false);
  assert.equal(change.includes("has no string-length or character-class predicate"), false);
});
