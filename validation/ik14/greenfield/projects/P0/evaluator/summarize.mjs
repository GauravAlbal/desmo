import { readFileSync } from "node:fs";

const text = readFileSync(process.argv[2], "utf8");
const ids = ["R1","R2","R3","R4","R5","R6","R7","R8","R9","R10"];
const seen = {};
for (const id of ids) seen[id] = "missing";
for (const line of text.split(/\n/)) {
  const match = line.match(/^(not )?ok \d+ - (R\d+)\b/);
  if (!match) continue;
  const id = match[2];
  if (!Object.prototype.hasOwnProperty.call(seen, id)) continue;
  if (match[1]) seen[id] = "fail";
  else if (seen[id] !== "fail") seen[id] = "pass";
}
const failed = Object.entries(seen).filter((pair) => pair[1] !== "pass").map((pair) => pair[0]);
const out = { requirements: seen, ok: failed.length === 0 };
process.stdout.write(JSON.stringify(out, null, 2) + "\n");
process.exit(out.ok ? 0 : 1);
