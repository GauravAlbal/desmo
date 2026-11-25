import { existsSync, readdirSync, readFileSync, type Dirent } from "node:fs";
import { basename, dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { skillStatus, type SkillStatus } from "./skill.js";

const SKIP_DIRS = new Set(["node_modules", ".git", "dist", "target"]);

export const DOCTOR_NOTE =
  "doctor reads manifests only; it does not execute adapters or fetch anything";

export type PathStatus = { path: string; exists: boolean };

export type ToolStatus = {
  name: string;
  path: string;
  exists: boolean;
  next_command: string;
};

export type Unavailable = {
  status: "unavailable";
  subject: string;
  path: string;
  next_command: string;
  line: string;
};

export type AdapterStatus = {
  manifest: string;
  adapter_id: string | null;
  engine: string | null;
  executable: PathStatus | null;
  executable_next_command: string | null;
  tools: ToolStatus[];
  read_error: string | null;
};

export type DoctorReport = {
  protocol: "ikaros.doctor.v1";
  executed_adapters: false;
  fetched: false;
  note: string;
  runtime: {
    package_name: "@desmo/runtime";
    package_version: string;
    ik_binary: string;
    ik_version_line: string;
    ik_version: string | null;
    protocol: string | null;
    versions_match: boolean;
  };
  project: string;
  checks_toml: { path: ".ik/checks.toml"; exists: boolean };
  machines: string[];
  scenarios: { path: "scenarios"; exists: boolean; files: string[] };
  adapters: AdapterStatus[];
  unavailable: Unavailable[];
  skill: SkillStatus;
};

type ToolDecl = { name: string; path: string; next_command?: string };

export function packageVersion(): string {
  const root = join(dirname(fileURLToPath(import.meta.url)), "..");
  const parsed = JSON.parse(readFileSync(join(root, "package.json"), "utf8")) as { version?: unknown };
  return typeof parsed.version === "string" ? parsed.version : "unknown";
}

export function parseIkVersion(line: string): { version: string | null; protocol: string | null } {
  const match = /^invariant-kernel\s+(\S+)\s+protocol\s+(\S+)/.exec(line.trim());
  if (!match) {
    return { version: null, protocol: null };
  }
  return { version: match[1], protocol: match[2] };
}

export function inspectProject(
  dir: string,
  ikBinary: string,
  ikVersionLine: string,
  runtimePackageVersion: string,
): DoctorReport {
  const project = resolve(dir);
  const parsed = parseIkVersion(ikVersionLine);
  const adapters = readAdapters(project);
  return {
    protocol: "ikaros.doctor.v1",
    executed_adapters: false,
    fetched: false,
    note: DOCTOR_NOTE,
    runtime: {
      package_name: "@desmo/runtime",
      package_version: runtimePackageVersion,
      ik_binary: ikBinary,
      ik_version_line: ikVersionLine.trim(),
      ik_version: parsed.version,
      protocol: parsed.protocol,
      versions_match: parsed.version === runtimePackageVersion,
    },
    project,
    checks_toml: {
      path: ".ik/checks.toml",
      exists: existsSync(join(project, ".ik", "checks.toml")),
    },
    machines: discoverMachines(project),
    scenarios: discoverScenarios(project),
    adapters,
    unavailable: unavailableLines(adapters),
    skill: skillStatus(project),
  };
}

export function renderDoctor(report: DoctorReport): string {
  const lines = [
    "desmo doctor",
    report.note,
    `runtime: ${report.runtime.package_name} ${report.runtime.package_version}`,
    `ik: ${report.runtime.ik_version_line}`,
    `ik binary: ${report.runtime.ik_binary}`,
    `versions_match: ${report.runtime.versions_match}`,
    ...(report.runtime.versions_match
      ? []
      : [
          `hint: @desmo/runtime ${report.runtime.package_version} requires matching ik ${report.runtime.package_version}; install the matching release or set IK_BINARY`,
        ]),
    `checks: ${report.checks_toml.path} ${report.checks_toml.exists ? "present" : "absent"}`,
    `machines: ${report.machines.length === 0 ? "none" : report.machines.join(", ")}`,
    scenarioLine(report),
    skillLine(report),
    ...adapterLines(report.adapters),
    ...report.unavailable.map((item) => item.line),
  ];
  return `${lines.join("\n")}\n`;
}


function skillLine(report: DoctorReport): string {
  if (!report.skill.installed) {
    return `skill: ${report.skill.path} absent`;
  }
  if (!report.skill.complete) {
    return `skill: ${report.skill.path} incomplete`;
  }
  return `skill: ${report.skill.path} present`;
}
function scenarioLine(report: DoctorReport): string {
  if (!report.scenarios.exists) {
    return "scenarios: absent";
  }
  if (report.scenarios.files.length === 0) {
    return "scenarios: none";
  }
  return `scenarios: ${report.scenarios.files.join(", ")}`;
}

function adapterLines(adapters: AdapterStatus[]): string[] {
  if (adapters.length === 0) {
    return ["adapters: none"];
  }
  return adapters.map(adapterLine);
}

function adapterLine(adapter: AdapterStatus): string {
  if (adapter.read_error) {
    return `adapter ${adapter.manifest}: unreadable (${adapter.read_error})`;
  }
  const executable = adapter.executable;
  const executableText = executable
    ? `${executable.path} ${executable.exists ? "present" : "absent"}`
    : "undeclared";
  return `adapter ${adapter.adapter_id ?? "unknown"} engine ${adapter.engine ?? "unknown"} executable ${executableText}`;
}

function discoverMachines(root: string): string[] {
  const found: string[] = [];
  walkMachines(root, root, found);
  found.sort();
  return found;
}

function walkMachines(root: string, current: string, found: string[]): void {
  for (const entry of readDir(current)) {
    if (SKIP_DIRS.has(entry.name)) {
      continue;
    }
    const full = join(current, entry.name);
    if (entry.isDirectory()) {
      walkMachines(root, full, found);
    } else if (entry.isFile() && entry.name.endsWith(".ik.toml")) {
      found.push(relative(root, full));
    }
  }
}

function discoverScenarios(root: string): DoctorReport["scenarios"] {
  const dir = join(root, "scenarios");
  if (!existsSync(dir)) {
    return { path: "scenarios", exists: false, files: [] };
  }
  const files = readDir(dir)
    .filter((entry) => entry.isFile() && entry.name.endsWith(".json"))
    .map((entry) => relative(root, join(dir, entry.name)))
    .sort();
  return { path: "scenarios", exists: true, files };
}

function readAdapters(root: string): AdapterStatus[] {
  const dir = join(root, ".ik", "adapters");
  if (!existsSync(dir)) {
    return [];
  }
  return readDir(dir)
    .filter((entry) => entry.isFile() && entry.name.endsWith(".json"))
    .map((entry) => relative(root, join(dir, entry.name)))
    .sort()
    .map((manifest) => readAdapter(root, manifest));
}

function readAdapter(root: string, manifest: string): AdapterStatus {
  let parsed: unknown;
  try {
    parsed = JSON.parse(readFileSync(join(root, manifest), "utf8"));
  } catch {
    return emptyAdapter(manifest, "malformed JSON");
  }
  if (!isRecord(parsed)) {
    return emptyAdapter(manifest, "manifest is not an object");
  }
  const executablePath = executableOf(parsed);
  const executableNext = stringField(isRecord(parsed.invocation) ? parsed.invocation.next_command : undefined);
  return {
    manifest,
    adapter_id: adapterIdOf(parsed),
    engine: engineOf(parsed),
    executable: executablePath
      ? { path: executablePath, exists: pathExists(root, executablePath) }
      : null,
    executable_next_command: executableNext,
    tools: toolsOf(parsed).map((tool) => ({
      name: tool.name,
      path: tool.path,
      exists: pathExists(root, tool.path),
      next_command: oneLine(tool.next_command) ?? `test -e ${shellSingle(tool.path)}`,
    })),
    read_error: null,
  };
}

function emptyAdapter(manifest: string, readError: string): AdapterStatus {
  return {
    manifest,
    adapter_id: null,
    engine: null,
    executable: null,
    executable_next_command: null,
    tools: [],
    read_error: readError,
  };
}

function unavailableLines(adapters: AdapterStatus[]): Unavailable[] {
  const lines: Unavailable[] = [];
  for (const adapter of adapters) {
    if (adapter.executable && !adapter.executable.exists) {
      const id = adapter.adapter_id ?? adapter.manifest;
      const next = adapter.executable_next_command ?? `test -e ${shellSingle(adapter.executable.path)}`;
      lines.push(unavailable(`adapter ${id} executable`, adapter.executable.path, next));
    }
    for (const tool of adapter.tools) {
      if (!tool.exists) {
        lines.push(unavailable(`engine tool ${tool.name}`, tool.path, tool.next_command));
      }
    }
  }
  return lines;
}

function unavailable(subject: string, path: string, nextCommand: string): Unavailable {
  return {
    status: "unavailable",
    subject,
    path,
    next_command: nextCommand,
    line: `unavailable: ${subject} ${path} — next: ${nextCommand}`,
  };
}

function readDir(dir: string): Dirent[] {
  try {
    return readdirSync(dir, { withFileTypes: true });
  } catch {
    return [];
  }
}

function pathExists(root: string, path: string): boolean {
  if (path.length === 0) {
    return false;
  }
  return existsSync(path.startsWith("/") ? path : resolve(root, path));
}

function adapterIdOf(doc: Record<string, unknown>): string | null {
  const adapter = doc.adapter;
  if (!isRecord(adapter)) {
    return null;
  }
  return stringField(adapter.id);
}

function engineOf(doc: Record<string, unknown>): string | null {
  const engine = doc.engine;
  if (typeof engine === "string") {
    return stringField(engine);
  }
  if (!isRecord(engine)) {
    return null;
  }
  return stringField(engine.id);
}

function executableOf(doc: Record<string, unknown>): string | null {
  const invocation = doc.invocation;
  if (!isRecord(invocation) || !Array.isArray(invocation.argv)) {
    return null;
  }
  return stringField(invocation.argv[0]);
}

function toolsOf(doc: Record<string, unknown>): ToolDecl[] {
  const collected = [...toolEntries(doc.tools), ...engineToolEntries(doc.engine)];
  const seen = new Set<string>();
  const tools: ToolDecl[] = [];
  for (const tool of collected) {
    const key = `${tool.name}\0${tool.path}`;
    if (seen.has(key)) {
      continue;
    }
    seen.add(key);
    tools.push(tool);
  }
  return tools;
}

function engineToolEntries(engine: unknown): ToolDecl[] {
  if (!isRecord(engine)) {
    return [];
  }
  return [...toolEntries(engine.tools), ...stringPaths(engine.tool_paths)];
}

function toolEntries(value: unknown): ToolDecl[] {
  if (!Array.isArray(value)) {
    return [];
  }
  const tools: ToolDecl[] = [];
  for (const item of value) {
    const tool = toolEntry(item);
    if (tool) {
      tools.push(tool);
    }
  }
  return tools;
}

function stringPaths(value: unknown): ToolDecl[] {
  if (!Array.isArray(value)) {
    return [];
  }
  const tools: ToolDecl[] = [];
  for (const item of value) {
    if (typeof item === "string" && item.length > 0) {
      tools.push({ name: basename(item), path: item });
    }
  }
  return tools;
}

function toolEntry(item: unknown): ToolDecl | null {
  if (typeof item === "string" && item.length > 0) {
    return { name: basename(item), path: item };
  }
  if (!isRecord(item) || typeof item.path !== "string" || item.path.length === 0) {
    return null;
  }
  const name = stringField(item.name) ?? basename(item.path);
  const next = stringField(item.next_command);
  return next ? { name, path: item.path, next_command: next } : { name, path: item.path };
}

function stringField(value: unknown): string | null {
  return typeof value === "string" && value.trim().length > 0 ? value : null;
}

function oneLine(value: string | undefined): string | null {
  if (!value) {
    return null;
  }
  const collapsed = value.replace(/[\r\n]+/g, " ").trim();
  return collapsed.length > 0 ? collapsed : null;
}

function shellSingle(path: string): string {
  return `'${path.replaceAll("'", `'\\''`)}'`;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
