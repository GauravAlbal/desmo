import { existsSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const require = createRequire(import.meta.url);

const PLATFORM_PACKAGES: Record<string, string> = {
  "darwin-arm64": "@desmo/darwin-arm64",
  "darwin-x64": "@desmo/darwin-x64",
  "linux-arm64": "@desmo/linux-arm64",
  "linux-x64": "@desmo/linux-x64",
};

export function locateBinary(explicit?: string): string {
  if (explicit) {
    return explicit;
  }
  if (process.env.IK_BINARY) {
    return process.env.IK_BINARY;
  }
  if (process.env.IK_BIN) {
    return process.env.IK_BIN;
  }
  const key = `${process.platform}-${process.arch}`;
  const pkg = PLATFORM_PACKAGES[key];
  const fromPackage = pkg ? platformBinary(pkg) : undefined;
  if (fromPackage) {
    return fromPackage;
  }
  const fromPath = which("ik");
  if (fromPath) {
    return fromPath;
  }
  throw new Error(
    `ik binary not found for ${key}. Install the matching @desmo/<platform> package, set IK_BINARY, or put ik on PATH. This SDK does not implement the kernel.`,
  );
}

/** The optional platform package may be absent; that is not an error. */
function platformBinary(pkg: string): string | undefined {
  try {
    const bin = join(dirname(require.resolve(`${pkg}/package.json`)), "bin", "ik");
    return existsSync(bin) ? bin : undefined;
  } catch {
    return undefined;
  }
}

function which(name: string): string | undefined {
  const pathEnv = process.env.PATH ?? "";
  const sep = process.platform === "win32" ? ";" : ":";
  for (const dir of pathEnv.split(sep)) {
    const candidate = join(dir, name);
    if (existsSync(candidate)) {
      return candidate;
    }
  }
  return undefined;
}

export function packageRoot(): string {
  return dirname(fileURLToPath(new URL("..", import.meta.url)));
}
