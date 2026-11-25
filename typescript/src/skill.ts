import { copyFileSync, existsSync, mkdirSync, readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { packageRoot } from "./assets.js";

export const SKILL_FILES = [
  "SKILL.md",
  "audit.md",
  "build.md",
  "change.md",
  "diagnose.md",
  "readout.md",
  "routing.json",
] as const;

export const SKILL_RELATIVE = join(".agents", "skills", "desmo");

export const DESMO_USAGE =
  "Usage: desmo demo orders | desmo demo stale-worker | desmo simulate <machine.ik.toml> [--out file.html] | desmo init --template job --language typescript [--out dir] | desmo doctor [--dir project] [--json] | desmo codegen <machine.ik.toml> [--out file] [--check] [--import-from specifier] | desmo trace <bundle.json> --out <file.html> | desmo skill install [--dir <project>] [--target <agent-skills-dir>] [--force]";

export type SkillStatus = {
  path: string;
  installed: boolean;
  complete: boolean;
  missing: string[];
};

export type InstallRequest = {
  projectDir: string;
  targetDir?: string;
  force: boolean;
};

export type InstallPlan = {
  destination: string;
  projectDir: string;
  copied: string[];
  unchanged: string[];
  conflicts: string[];
};

export function packagedSkillDir(): string {
  const packed = join(packageRoot(), "skills", "desmo");
  if (existsSync(join(packed, "SKILL.md"))) {
    return packed;
  }
  const repo = join(packageRoot(), "..", "skills", "desmo");
  if (existsSync(join(repo, "SKILL.md"))) {
    return repo;
  }
  throw new Error("packaged Desmo skill is missing (expected skills/desmo/SKILL.md)");
}

export function skillStatus(projectDir: string): SkillStatus {
  const dir = join(resolve(projectDir), SKILL_RELATIVE);
  const missing = SKILL_FILES.filter((name) => !existsSync(join(dir, name)));
  return {
    path: SKILL_RELATIVE,
    installed: existsSync(join(dir, "SKILL.md")),
    complete: missing.length === 0,
    missing,
  };
}

export function unknownSkillFlag(args: string[]): string | null {
  const valued = new Set(["--dir", "--target"]);
  for (let i = 0; i < args.length; i += 1) {
    const token = args[i];
    if (!token.startsWith("-")) {
      continue;
    }
    if (token === "--force" || token === "-h" || token === "--help") {
      continue;
    }
    if (!valued.has(token)) {
      return token;
    }
    i += 1;
  }
  return null;
}

export function planInstall(sourceDir: string, request: InstallRequest): InstallPlan {
  const destination = request.targetDir
    ? join(resolve(request.targetDir), "desmo")
    : join(resolve(request.projectDir), SKILL_RELATIVE);
  const copied: string[] = [];
  const unchanged: string[] = [];
  const conflicts: string[] = [];
  for (const name of SKILL_FILES) {
    classifyFile(sourceDir, destination, name, request.force, copied, unchanged, conflicts);
  }
  return { destination, projectDir: resolve(request.projectDir), copied, unchanged, conflicts };
}

function classifyFile(
  sourceDir: string,
  destination: string,
  name: string,
  force: boolean,
  copied: string[],
  unchanged: string[],
  conflicts: string[],
): void {
  const dest = join(destination, name);
  if (!existsSync(dest)) {
    copied.push(name);
    return;
  }
  const same = readFileSync(dest).equals(readFileSync(join(sourceDir, name)));
  if (same) {
    unchanged.push(name);
  } else if (force) {
    copied.push(name);
  } else {
    conflicts.push(name);
  }
}

export function applyInstall(sourceDir: string, plan: InstallPlan): void {
  if (plan.conflicts.length > 0) {
    return;
  }
  mkdirSync(plan.destination, { recursive: true });
  for (const name of plan.copied) {
    copyFileSync(join(sourceDir, name), join(plan.destination, name));
  }
}

export function renderInstall(plan: InstallPlan): string {
  if (plan.conflicts.length > 0) {
    return `${plan.conflicts
      .map((name) => `refusing to overwrite ${join(plan.destination, name)}; rerun with --force`)
      .join("\n")}\n`;
  }
  const head = plan.copied.length === 0 ? `unchanged ${plan.destination}` : `installed ${plan.destination}`;
  return [
    head,
    `next: point the coding agent at ${join(plan.destination, "SKILL.md")}`,
    `next: desmo doctor --dir ${plan.projectDir}`,
    "next: open SKILL.md and follow build.md, change.md, or diagnose.md",
    "",
  ].join("\n");
}
