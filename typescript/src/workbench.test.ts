import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { chmodSync, existsSync, mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import { bundledMachine } from "./assets.js";

const root = fileURLToPath(new URL("..", import.meta.url));
const cliJs = join(root, "dist", "cli.js");
const packageVersion = JSON.parse(readFileSync(join(root, "package.json"), "utf8")).version as string;

type Run = { status: number | null; stdout: string; stderr: string };

function runCli(args: string[], cwd: string, env: NodeJS.ProcessEnv = process.env): Run {
  const result = spawnSync(process.execPath, [cliJs, ...args], { cwd, env, encoding: "utf8" });
  return { status: result.status, stdout: result.stdout ?? "", stderr: result.stderr ?? "" };
}

function runIk(args: string[], cwd: string): Run {
  const binary = process.env.IK_BINARY || process.env.IK_BIN || join(root, "..", "target", "debug", "ik");
  const result = spawnSync(binary, args, { cwd, encoding: "utf8" });
  return { status: result.status, stdout: result.stdout ?? "", stderr: result.stderr ?? "" };
}

test("doctor reports versions and a missing tool without executing a manifest", () => {
  const dir = mkdtempSync(join(tmpdir(), "desmo-doctor-"));
  try {
    const marker = join(dir, "marker");
    const script = join(dir, "write-marker.sh");
    writeFileSync(script, `#!/bin/sh\nprintf ran > '${marker}'\n`);
    chmodSync(script, 0o755);
    const fakeIk = join(dir, "fake-ik");
    writeFileSync(fakeIk, "#!/bin/sh\nprintf '%s\\n' 'invariant-kernel 9.9.9 protocol 1.0'\n");
    chmodSync(fakeIk, 0o755);
    mkdirSync(join(dir, ".ik", "adapters"), { recursive: true });
    writeFileSync(join(dir, ".ik", "adapters", "bad.json"), "{");
    const missingExec = join(dir, "missing-adapter");
    writeFileSync(
      join(dir, ".ik", "adapters", "missing.json"),
      JSON.stringify({
        adapter: { id: "missing-adapter", version: "1" },
        invocation: { argv: [missingExec], next_command: "install-optional-adapter" },
        engine: { id: "missing-engine", version_constraint: "0" },
      }),
    );
    writeFileSync(
      join(dir, ".ik", "adapters", "marker.json"),
      JSON.stringify({
        protocol: "ik.analysis.adapter.v1",
        adapter: { id: "marker-adapter", version: "1" },
        invocation: { argv: [script], executable_sha256: "not-used" },
        engine: {
          id: "marker-engine",
          version_constraint: "0",
          tools: [{ name: "marker-tool", path: join(dir, "missing-tool"), next_command: script }],
        },
      }),
    );
    const env = { ...process.env, IK_BINARY: fakeIk, IK_BIN: "" };
    const json = runCli(["doctor", "--dir", dir, "--json"], dir, env);
    assert.equal(json.status, 0, json.stderr);
    assert.equal(existsSync(marker), false);
    const report = JSON.parse(json.stdout);
    assert.equal(report.executed_adapters, false);
    assert.equal(report.fetched, false);
    assert.match(report.note, /does not execute adapters or fetch anything/);
    assert.equal(report.runtime.package_version, packageVersion);
    assert.equal(report.runtime.ik_version, "9.9.9");
    assert.equal(report.runtime.protocol, "1.0");
    assert.equal(report.runtime.versions_match, false);
    assert.equal(report.checks_toml.exists, false);
    const markerAdapter = report.adapters.find((item: { manifest: string }) => item.manifest.endsWith("marker.json"));
    assert.equal(markerAdapter.adapter_id, "marker-adapter");
    assert.equal(markerAdapter.engine, "marker-engine");
    assert.equal(markerAdapter.executable.exists, true);
    assert.equal(markerAdapter.tools[0].exists, false);
    assert.equal(report.unavailable.length, 2);
    const toolLine = report.unavailable.find((item: { subject: string }) => item.subject === "engine tool marker-tool");
    assert.equal(toolLine.status, "unavailable");
    assert.equal(toolLine.next_command, script);
    assert.match(toolLine.line, /^unavailable: engine tool marker-tool /);
    const missingLine = report.unavailable.find((item: { subject: string }) => item.subject.includes("missing-adapter"));
    assert.equal(missingLine.status, "unavailable");
    assert.equal(missingLine.next_command, "install-optional-adapter");
    assert.equal(existsSync(missingExec), false);
    const bad = report.adapters.find((item: { manifest: string }) => item.manifest.endsWith("bad.json"));
    assert.equal(bad.read_error, "malformed JSON");
    const text = runCli(["doctor", "--dir", dir], dir, env);
    assert.equal(text.status, 0, text.stderr);
    assert.match(text.stdout, /does not execute adapters or fetch anything/);
    assert.match(text.stdout, /unavailable: engine tool marker-tool /);
    assert.match(text.stdout, /next: /);
    assert.equal(existsSync(marker), false);
    const missing = runCli(["doctor", "--dir", dir], dir, {
      ...process.env,
      IK_BINARY: join(dir, "no-such-ik"),
      IK_BIN: "",
    });
    assert.equal(missing.status, 1);
    assert.match(missing.stderr, /ik runtime not found/);
    assert.equal(existsSync(marker), false);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("job scaffold keeps the existing project and the documented commands pass", () => {
  const dir = mkdtempSync(join(tmpdir(), "desmo-job-scaffold-"));
  try {
    const init = runCli(["init", "--template", "job", "--language", "typescript", "--out", dir], tmpdir());
    assert.equal(init.status, 0, init.stderr);
    assert.match(init.stdout, /wrote /);
    const readme = readFileSync(join(dir, "README.md"), "utf8");
    assert.match(readme, /Runtime remains authoritative/);
    assert.match(readme, /authority/i);
    const checkCmd = "ik check job.ik.toml --profile .ik/checks.toml";
    const scenarioCmd = "ik scenario run job.ik.toml scenarios/normal.json --out .ik/runs/normal";
    assert.ok(readme.includes(checkCmd));
    assert.ok(readme.includes(scenarioCmd));
    assert.equal(readFileSync(join(dir, "job.ik.toml"), "utf8"), readFileSync(bundledMachine(), "utf8"));
    const pkg = JSON.parse(readFileSync(join(dir, "package.json"), "utf8"));
    assert.equal(pkg.scripts.test, "node --test job.test.mjs");
    assert.equal(pkg.dependencies["@desmo/runtime"], "0.5.0");
    assert.match(readFileSync(join(dir, "job.test.mjs"), "utf8"), /StaleCapabilityError/);
    assert.equal(existsSync(join(dir, ".ik", "checks.toml")), true);
    assert.match(readFileSync(join(dir, ".ik", "checks.toml"), "utf8"), /reach_terminal/);
    assert.match(readFileSync(join(dir, ".ik", "checks.toml"), "utf8"), /no_stuck_states/);
    const again = runCli(["init", "--template", "job", "--out", dir], tmpdir());
    assert.equal(again.status, 1);
    assert.match(again.stderr, /refusing to overwrite/);

    const doctor = runCli(["doctor", "--dir", dir, "--json"], dir);
    assert.equal(doctor.status, 0, doctor.stderr);
    const report = JSON.parse(doctor.stdout);
    assert.equal(report.checks_toml.exists, true);
    assert.ok(report.machines.includes("job.ik.toml"));
    assert.ok(report.scenarios.files.includes("scenarios/normal.json"));
    assert.ok(report.scenarios.files.includes("scenarios/stale-handle.json"));
    assert.equal(report.executed_adapters, false);
    assert.equal(report.fetched, false);
    assert.equal(report.runtime.package_version, packageVersion);
    assert.match(report.runtime.ik_version_line, /^invariant-kernel \S+ protocol \S+/);

    const check = runIk(["check", "job.ik.toml", "--profile", ".ik/checks.toml", "--json"], dir);
    assert.equal(check.status, 0, check.stderr + check.stdout);
    const checked = JSON.parse(check.stdout);
    assert.equal(checked.judgment.verdict, "accept");
    assert.equal(checked.judgment.status, "accepted");
    const verified = checked.profile.goals.find((goal: { id: string }) => goal.id === "verified");
    assert.equal(verified.kind, "reach_terminal");
    assert.equal(verified.status, "witness_found");
    const stuck = checked.profile.goals.find((goal: { kind: string }) => goal.kind === "no_stuck_states");
    assert.equal(stuck.status, "no_stuck_states_found");

    const normal = runIk(
      ["scenario", "run", "job.ik.toml", "scenarios/normal.json", "--out", ".ik/runs/normal"],
      dir,
    );
    assert.equal(normal.status, 0, normal.stderr + normal.stdout);
    assert.match(normal.stdout, /all expectations met/);

    const stale = runIk(
      ["scenario", "run", "job.ik.toml", "scenarios/stale-handle.json", "--out", ".ik/runs/stale-handle"],
      dir,
    );
    assert.equal(stale.status, 0, stale.stderr + stale.stdout);
    const result = JSON.parse(readFileSync(join(dir, ".ik/runs/stale-handle/result.json"), "utf8"));
    assert.equal(result.all_expectations_met, true);
    const staleStep = result.steps.find((step: { label: string }) => step.label === "stale");
    assert.equal(staleStep.expectation, "refused:IK_CAPABILITY_STALE");
    assert.equal(staleStep.verdict, "pass");

    const machinePath = join(dir, "job.ik.toml");
    const original = readFileSync(machinePath, "utf8");
    const refused = original.replace(
      'when = "$receipt_artifact_digest == artifact_digest && $receipt_baseline_digest == baseline_digest"',
      'when = "false == true"',
    );
    assert.notEqual(refused, original);
    writeFileSync(machinePath, refused);
    const failed = runIk(["check", "job.ik.toml", "--profile", ".ik/checks.toml", "--json"], dir);
    assert.equal(failed.status, 1, failed.stdout + failed.stderr);
    const failedDoc = JSON.parse(failed.stdout);
    assert.equal(failedDoc.judgment.verdict, "refused");
    assert.equal(failedDoc.judgment.status, "missing_evidence");
    const failedGoal = failedDoc.profile.goals.find((goal: { id: string }) => goal.id === "verified");
    assert.notEqual(failedGoal.status, "witness_found");
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});
