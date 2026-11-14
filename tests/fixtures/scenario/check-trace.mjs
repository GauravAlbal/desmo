import { mkdtempSync, readFileSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const repo = join(here, "../../..");
const dir = mkdtempSync(join(tmpdir(), "ik-trace-parser-"));
try {
  writeFileSync(join(dir, "guards.ts"), readFileSync(join(repo, "typescript/src/guards.ts")));
  const trace = readFileSync(join(repo, "typescript/src/trace.ts"), "utf8").replace(
    'from "./guards.js"',
    'from "./guards.ts"',
  );
  writeFileSync(join(dir, "trace.ts"), trace);
  const parser = await import(pathToFileURL(join(dir, "trace.ts")).href);
  parser.parseTraceBundle(JSON.parse(readFileSync(process.argv[2], "utf8")));
} finally {
  rmSync(dir, { recursive: true, force: true });
}
